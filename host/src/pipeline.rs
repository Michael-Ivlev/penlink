//! Capture (Windows.Graphics.Capture on one HWND) -> I420 -> openh264 -> WebRTC track.
//!
//! Runs on one dedicated OS thread per session.

use crate::encode::{H264Encoder, DEFAULT_BITRATE, MAX_FPS};
use crate::frame::{Converter, I420};
use crate::geom::{even, Rect};
use crate::input::Shared;
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
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

const KEYFRAME_INTERVAL: Duration = Duration::from_secs(4);
const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(16_000); // ~60 fps cap
const IDLE_RESEND: Duration = Duration::from_millis(450); // >= ~2 fps even when nothing changes

#[derive(Debug)]
pub enum PipelineEvent {
    /// The captured window no longer exists.
    WindowClosed,
    Error(String),
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
    target: Hwnd,
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
    target: Hwnd,
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
    let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd(target))? };
    let mut pool_size = item.Size()?;
    let fmt = DirectXPixelFormat::B8G8R8A8UIntNormalized;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d_dev, fmt, 3, pool_size)?;
    let (tx, rx) = channel::<()>();
    let closed = Arc::new(AtomicBool::new(false));
    {
        let tx = tx.clone();
        pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
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
    session.StartCapture()?;
    tracing::info!("capture started for window {target:#x}, item size {}x{}", pool_size.Width, pool_size.Height);

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

    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            break;
        }
        if closed.load(Ordering::SeqCst) || !is_window(target) {
            let _ = events.send(PipelineEvent::WindowClosed);
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

        // Pull the newest captured frame (drop older ones).
        let mut newest = None;
        while let Ok(f) = pool.TryGetNextFrame() {
            newest = Some(f);
        }
        let mut converted = false;
        if let Some(frame) = newest {
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
            let crop = client_crop(target, desc.Width as i32, desc.Height as i32);
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
        let _ = signalled;

        if !have_frame {
            continue;
        }
        let idle_due = last_encode.elapsed() >= IDLE_RESEND;
        let key_due = last_idr.elapsed() >= KEYFRAME_INTERVAL;
        if !(dirty || idle_due || key_due || ctl.want_keyframe.load(Ordering::SeqCst)) {
            continue;
        }
        // Frame-rate cap: leave `dirty` set so the newest content goes out right after.
        if converted || dirty {
            let since = last_encode.elapsed();
            if since < MIN_FRAME_INTERVAL && !ctl.want_keyframe.load(Ordering::SeqCst) {
                std::thread::sleep(MIN_FRAME_INTERVAL - since);
            }
        }

        if ctl.want_keyframe.swap(false, Ordering::SeqCst) || key_due {
            enc.force_keyframe();
            last_idr = Instant::now();
        }
        let t1 = Instant::now();
        let out = enc.encode(&yuv)?;
        ctl.stats.encode_us.fetch_add(t1.elapsed().as_micros() as u64, Ordering::Relaxed);
        last_encode = Instant::now();
        dirty = false;
        if let Some(e) = out {
            let now = Instant::now();
            let dur = now.duration_since(last_sample).clamp(Duration::from_millis(1), Duration::from_secs(1));
            last_sample = now;
            ctl.stats.frames.fetch_add(1, Ordering::Relaxed);
            ctl.stats.bytes.fetch_add(e.data.len() as u64, Ordering::Relaxed);
            let sample = Sample { data: Bytes::from(e.data), duration: dur, ..Default::default() };
            if let Err(err) = rt.block_on(track.write_sample(&sample)) {
                tracing::debug!("write_sample: {err}");
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
