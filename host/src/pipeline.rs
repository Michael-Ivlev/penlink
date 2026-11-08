//! Capture (Windows.Graphics.Capture on one HWND) -> I420 -> openh264 -> WebRTC track.
//!
//! Runs on one dedicated OS thread per session.

use crate::encode::{H264Encoder, DEFAULT_BITRATE, MAX_FPS};
use crate::frame::{Converter, I420};
use crate::geom::{even, Rect};
use crate::input::{InputTarget, Shared};
use crate::winutil::{client_rect_screen, frame_bounds, hwnd, is_window, Hwnd};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;
use webrtc::media::Sample;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use windows::core::{factory, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

const KEYFRAME_INTERVAL: Duration = Duration::from_secs(4);
const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(7_500); // ~120 fps cap
const IDLE_RESEND: Duration = Duration::from_millis(450); // >= ~2 fps even when nothing changes

/// What is captured.
#[derive(Debug, Clone, Copy)]
pub enum CaptureSource {
    Window(Hwnd),
    /// A whole (virtual) monitor. `watch` is the app window: when it is gone the session ends.
    Monitor { hmonitor: isize, rect: crate::geom::Rect, watch: Hwnd },
}

impl CaptureSource {
    fn watched_window(&self) -> Hwnd {
        match self {
            CaptureSource::Window(h) => *h,
            CaptureSource::Monitor { watch, .. } => *watch,
        }
    }
}

#[derive(Debug)]
pub enum PipelineEvent {
    /// The captured window no longer exists.
    WindowClosed,
    Error(String),
    /// Monitor mode: the streamed area changed size (taskbar appeared, DPI changed).
    Window(crate::session::WindowReport),
}

#[derive(Default)]
pub struct Stats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub convert_us: AtomicU64,
    pub encode_us: AtomicU64,
}

pub struct PipelineCtl {
    pub want_keyframe: AtomicBool,
    pub stop: AtomicBool,
    pub stats: Stats,
}

impl PipelineCtl {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { want_keyframe: AtomicBool::new(true), stop: AtomicBool::new(false), stats: Stats::default() })
    }
}

pub struct PipelineHandle {
    pub ctl: Arc<PipelineCtl>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PipelineHandle {
    pub fn stop(&mut self) {
        self.ctl.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PipelineHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn(
    target: CaptureSource,
    shared: Arc<Shared>,
    track: Arc<TrackLocalStaticSample>,
    rt: Handle,
    events: UnboundedSender<PipelineEvent>,
) -> PipelineHandle {
    let ctl = PipelineCtl::new();
    let ctl2 = ctl.clone();
    let thread = std::thread::Builder::new()
        .name("penlink-capture".into())
        .spawn(move || {
            if let Err(e) = run(target, shared, track, rt, &events, &ctl2) {
                tracing::error!("capture pipeline failed: {e:#}");
                let _ = events.send(PipelineEvent::Error(format!("{e}")));
            }
        })
        .expect("spawn capture thread");
    PipelineHandle { ctl, thread: Some(thread) }
}

struct Staging {
    tex: ID3D11Texture2D,
    w: u32,
    h: u32,
}

fn create_staging(dev: &ID3D11Device, w: u32, h: u32, fmt: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT) -> Result<Staging> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: fmt,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { dev.CreateTexture2D(&desc, None, Some(&mut tex))? };
    Ok(Staging { tex: tex.ok_or_else(|| anyhow!("no staging texture"))?, w, h })
}

fn run(
    target: CaptureSource,
    shared: Arc<Shared>,
    track: Arc<TrackLocalStaticSample>,
    rt: Handle,
    events: &UnboundedSender<PipelineEvent>,
    ctl: &PipelineCtl,
) -> Result<()> {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    // --- D3D11 device
    let mut device: Option<ID3D11Device> = None;
    let mut ctx: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut ctx),
        )?;
    }
    let device = device.ok_or_else(|| anyhow!("no D3D11 device"))?;
    let ctx = ctx.ok_or_else(|| anyhow!("no D3D11 context"))?;
    if let Ok(mt) = device.cast::<ID3D11Multithread>() {
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }
    }
    let dxgi: IDXGIDevice = device.cast()?;
    let d3d_dev: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;

    // --- capture item / pool / session
    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let item: GraphicsCaptureItem = match target {
        CaptureSource::Window(h) => unsafe { interop.CreateForWindow(hwnd(h))? },
        CaptureSource::Monitor { hmonitor, rect, .. } => {
            // Windows may still be settling a monitor that just changed its mode: the handle
            // can be stale for a moment. Look the monitor up again by its rectangle and retry.
            let mut h = hmonitor;
            let mut tries = 0;
            loop {
                match unsafe { interop.CreateForMonitor(HMONITOR(h as *mut std::ffi::c_void)) } {
                    Ok(i) => break i,
                    Err(e) if tries < 15 => {
                        tries += 1;
                        tracing::debug!("capture of monitor handle {h:#x} failed ({e}), retry {tries}");
                        std::thread::sleep(Duration::from_millis(150));
                        if let Some(m) = crate::vdisplay::enum_monitors().into_iter().find(|m| m.rect == rect) {
                            h = m.hmonitor;
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    };
    let watched = target.watched_window();
    let mut pool_size = item.Size()?;
    let fmt = DirectXPixelFormat::B8G8R8A8UIntNormalized;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d_dev, fmt, 3, pool_size)?;
    let (tx, rx) = channel::<()>();
    let closed = Arc::new(AtomicBool::new(false));
    let arrival = Arc::new(AtomicU64::new(0));
    {
        let tx = tx.clone();
        let arrival = arrival.clone();
        pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
            arrival.store(crate::latency::now_us(), Ordering::SeqCst);
            let _ = tx.send(());
            Ok(())
        }))?;
    }
    {
        let closed = closed.clone();
        let tx = tx.clone();
        item.Closed(&TypedEventHandler::new(move |_, _| {
            closed.store(true, Ordering::SeqCst);
            let _ = tx.send(());
            Ok(())
        }))?;
    }
    let session = pool.CreateCaptureSession(&item)?;
    if let Err(e) = session.SetIsBorderRequired(false) {
        tracing::debug!("could not disable capture border: {e}");
    }
    let _ = session.SetIsCursorCaptureEnabled(true);
    // Deliver every composed frame at once (the default throttles to the monitor's refresh interval).
    if let Err(e) = session.SetMinUpdateInterval(windows::Foundation::TimeSpan { Duration: 10_000 }) {
        tracing::debug!("could not set the capture update interval: {e}");
    }
    session.StartCapture()?;
    tracing::info!("capture started for {target:?}, item size {}x{}", pool_size.Width, pool_size.Height);

    // --- encoder state
    let (mut vw, mut vh) = {
        let (w, h) = shared.video();
        (even(w as u32) as usize, even(h as u32) as usize)
    };
    let mut yuv = I420::new(vw, vh);
    let mut conv = Converter::new();
    let mut enc = H264Encoder::new(vw, vh, DEFAULT_BITRATE)?;
    let mut staging: Option<Staging> = None;
    let mut have_frame = false;
    let mut last_encode = Instant::now() - Duration::from_secs(1);
    let mut last_sample = Instant::now();
    let mut last_idr = Instant::now();
    let mut dirty = false; // new frame converted but not yet encoded
    let mut last_stats = Instant::now();
    let mut encode_errors = 0u32;
    let playout = [webrtc::rtp::extension::HeaderExtension::PlayoutDelay(webrtc::rtp::extension::playout_delay_extension::PlayoutDelayExtension { min_delay: 0, max_delay: 0 })];
    let mut frame_arrived_us = 0u64;

    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            break;
        }
        if !is_window(watched) {
            let _ = events.send(PipelineEvent::WindowClosed);
            break;
        }
        if closed.load(Ordering::SeqCst) {
            // A window capture item closes with its window; a monitor item closes when the
            // monitor goes away (driver stopped, monitor removed behind our back).
            let ev = match target {
                CaptureSource::Window(_) => PipelineEvent::WindowClosed,
                CaptureSource::Monitor { .. } => PipelineEvent::Error("virtual monitor lost".into()),
            };
            let _ = events.send(ev);
            break;
        }
        // Wait for a frame, a keepalive timeout, or a stop poll tick.
        let wait = if dirty {
            Duration::from_millis(1)
        } else {
            IDLE_RESEND.saturating_sub(last_encode.elapsed()).max(Duration::from_millis(1)).min(Duration::from_millis(100))
        };
        let signalled = match rx.recv_timeout(wait) {
            Ok(()) => true,
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        while rx.try_recv().is_ok() {}

        // Output size changed (resize message)?
        let (nw, nh) = {
            let (w, h) = shared.video();
            (even(w as u32) as usize, even(h as u32) as usize)
        };
        if (nw, nh) != (vw, vh) {
            vw = nw;
            vh = nh;
            yuv = I420::new(vw, vh);
            conv = Converter::new();
            enc = H264Encoder::new(vw, vh, DEFAULT_BITRATE)?;
            ctl.want_keyframe.store(true, Ordering::SeqCst);
            if have_frame {
                dirty = true; // re-render the last frame at the new size below
            }
        }

        // Frame-rate cap: wait BEFORE taking the newest frame, so what is encoded is as fresh as
        // possible (the frames that arrive meanwhile replace each other in the pool).
        if (signalled || dirty) && !ctl.want_keyframe.load(Ordering::SeqCst) {
            let since = last_encode.elapsed();
            if since < MIN_FRAME_INTERVAL {
                std::thread::sleep(MIN_FRAME_INTERVAL - since);
            }
        }

        // Pull the newest captured frame (drop older ones).
        let mut newest = None;
        while let Ok(f) = pool.TryGetNextFrame() {
            newest = Some(f);
        }
        let mut converted = false;
        if let Some(frame) = newest {
            frame_arrived_us = arrival.load(Ordering::SeqCst);
            let content = frame.ContentSize()?;
            let t0 = Instant::now();
            let surface = frame.Surface()?;
            let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
            let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { tex.GetDesc(&mut desc) };
            if staging.as_ref().map(|s| (s.w, s.h)) != Some((desc.Width, desc.Height)) {
                staging = Some(create_staging(&device, desc.Width, desc.Height, desc.Format)?);
            }
            let st = staging.as_ref().unwrap();
            unsafe {
                ctx.CopyResource(&st.tex, &tex);
            }
            drop(surface);
            drop(frame);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe { ctx.Map(&st.tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
            let stride = mapped.RowPitch as usize;
            let src = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, stride * desc.Height as usize) };
            let crop = match (&target, &shared.target) {
                (CaptureSource::Window(h), _) => client_crop(*h, desc.Width as i32, desc.Height as i32),
                (CaptureSource::Monitor { .. }, InputTarget::Monitor(m)) => monitor_crop(m.crop(), desc.Width as i32, desc.Height as i32),
                _ => Rect::new(0, 0, desc.Width as i32, desc.Height as i32),
            };
            conv.convert(src, stride, crop, &mut yuv);
            unsafe { ctx.Unmap(&st.tex, 0) };
            ctl.stats.convert_us.fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
            have_frame = true;
            converted = true;
            // keep the pool matched to the window size
            if content.Width != pool_size.Width || content.Height != pool_size.Height {
                pool_size = SizeInt32 { Width: content.Width.max(1), Height: content.Height.max(1) };
                let _ = pool.Recreate(&d3d_dev, fmt, 3, pool_size);
            }
            dirty = true;
        }

        if !have_frame {
            continue;
        }
        let idle_due = last_encode.elapsed() >= IDLE_RESEND;
        let key_due = last_idr.elapsed() >= KEYFRAME_INTERVAL;
        if !(dirty || idle_due || key_due || ctl.want_keyframe.load(Ordering::SeqCst)) {
            continue;
        }
        if ctl.want_keyframe.swap(false, Ordering::SeqCst) || key_due {
            enc.force_keyframe();
            last_idr = Instant::now();
        }
        let t1 = Instant::now();
        let enc_start_us = crate::latency::now_us();
        let out = match enc.encode(&yuv) {
            Ok(o) => {
                encode_errors = 0;
                o
            }
            Err(e) => {
                // One bad frame must not end the session: start a fresh encoder and carry on;
                // give up only if it keeps failing.
                encode_errors += 1;
                tracing::warn!("encode failed ({encode_errors}): {e}");
                if encode_errors >= 5 {
                    return Err(e);
                }
                enc = H264Encoder::new(vw, vh, DEFAULT_BITRATE)?;
                ctl.want_keyframe.store(true, Ordering::SeqCst);
                dirty = true;
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        ctl.stats.encode_us.fetch_add(t1.elapsed().as_micros() as u64, Ordering::Relaxed);
        last_encode = Instant::now();
        dirty = false;
        let enc_end_us = crate::latency::now_us();
        if let Some(e) = out {
            let now = Instant::now();
            let dur = now.duration_since(last_sample).clamp(Duration::from_millis(1), Duration::from_secs(1));
            last_sample = now;
            ctl.stats.frames.fetch_add(1, Ordering::Relaxed);
            ctl.stats.bytes.fetch_add(e.data.len() as u64, Ordering::Relaxed);
            let sample = Sample { data: Bytes::from(e.data), duration: dur, ..Default::default() };
            if let Err(err) = rt.block_on(track.write_sample_with_extensions(&sample, &playout)) {
                tracing::debug!("write_sample: {err}");
            }
            let st = crate::latency::FrameStages {
                arrived_us: frame_arrived_us,
                encode_start_us: enc_start_us,
                encode_end_us: enc_end_us,
                sent_us: crate::latency::now_us(),
            };
            if let Some(rec) = crate::latency::PROBE.frame_sent(st) {
                tracing::debug!(
                    target: "penlink_host::latency",
                    "host stages: recv->inject {:.1} ms, inject->capture {:.1} ms, capture->encode {:.1} ms, encode {:.1} ms, send {:.1} ms, total {:.1} ms",
                    rec.recv_to_inject as f64 / 1e3, rec.inject_to_capture as f64 / 1e3, rec.capture_to_encode_start as f64 / 1e3,
                    rec.encode as f64 / 1e3, rec.send as f64 / 1e3, rec.total_us() as f64 / 1e3
                );
                if let Some(tx) = shared.lat_tx.lock().unwrap().as_ref() {
                    let _ = tx.send(rec.json());
                }
            }
        }
        if last_stats.elapsed() >= Duration::from_secs(5) {
            let secs = last_stats.elapsed().as_secs_f64();
            let f = ctl.stats.frames.swap(0, Ordering::Relaxed);
            let b = ctl.stats.bytes.swap(0, Ordering::Relaxed);
            let cu = ctl.stats.convert_us.swap(0, Ordering::Relaxed);
            let eu = ctl.stats.encode_us.swap(0, Ordering::Relaxed);
            tracing::debug!(
                "pipeline: {:.1} fps, {:.2} Mbps, convert {:.1} ms, encode {:.1} ms (per frame)",
                f as f64 / secs,
                b as f64 * 8.0 / secs / 1e6,
                if f > 0 { cu as f64 / f as f64 / 1000.0 } else { 0.0 },
                if f > 0 { eu as f64 / f as f64 / 1000.0 } else { 0.0 }
            );
            last_stats = Instant::now();
        }
        let _ = MAX_FPS;
    }

    let _ = session.Close();
    let _ = pool.Close();
    Ok(())
}

/// The streamed part of a monitor inside the captured texture; the whole texture when the
/// requested part does not fit.
fn monitor_crop(want: Rect, tex_w: i32, tex_h: i32) -> Rect {
    let full = Rect::new(0, 0, tex_w, tex_h);
    let x = want.x.clamp(0, tex_w - 2);
    let y = want.y.clamp(0, tex_h - 2);
    let w = want.w.min(tex_w - x);
    let h = want.h.min(tex_h - y);
    if w < 2 || h < 2 {
        return full;
    }
    Rect::new(x, y, w, h)
}

/// The window's client area inside the captured texture (which covers the window's visible
/// frame). Falls back to the whole texture.
fn client_crop(h: Hwnd, tex_w: i32, tex_h: i32) -> Rect {
    let full = Rect::new(0, 0, tex_w, tex_h);
    let (Some(c), Some(f)) = (client_rect_screen(h), frame_bounds(h)) else { return full };
    let x = (c.x - f.x).max(0);
    let y = (c.y - f.y).max(0);
    let w = c.w.min(tex_w - x);
    let hh = c.h.min(tex_h - y);
    if w < 2 || hh < 2 {
        return full;
    }
    Rect::new(x, y, w, hh)
}
