//! One opened app: window + capture pipeline + WebRTC peer connection + input thread.

use crate::geom::{client_size_for, clamp_video, Rect};
use crate::input::{InputTarget, Injector, MonitorTarget, Shared};
use crate::launcher::Claim;
use crate::pipeline::{self, CaptureSource, PipelineEvent, PipelineHandle};
use crate::protocol::{InputMsg, StreamMode};
use crate::sizing::{decide_resize, plan_sizes, MonitorSpec, ResizeAction, SizeRequest};
use crate::vdmanager::{VMonitor, VdManager, GRACE};
use crate::winutil::{self, Hwnd};
use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice::network_type::NetworkType;
use webrtc::ice::udp_network::{EphemeralUDP, UDPNetwork};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp_transceiver::RTCPFeedback;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

pub const H264_FMTP: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
/// Fixed UDP range for ICE so a firewall rule is easy to write.
pub const ICE_UDP_MIN: u16 = 47801;
pub const ICE_UDP_MAX: u16 = 48800;

pub enum InputCmd {
    Msg(InputMsg),
    Stop,
}

pub struct Session {
    pub id: String,
    pub app_id: String,
    pub hwnd: Hwnd,
    pub title: String,
    shared: Arc<Shared>,
    pc: Arc<RTCPeerConnection>,
    pipeline: StdMutex<Option<PipelineHandle>>,
    input_tx: std_mpsc::Sender<InputCmd>,
    input_thread: StdMutex<Option<std::thread::JoinHandle<()>>>,
    remote_set: AtomicBool,
    pending_ice: Mutex<Vec<RTCIceCandidateInit>>,
    closed: AtomicBool,
    /// Serialises window resizes so the last request wins.
    resize_lock: Mutex<()>,
    /// Keeps the window reserved for this session until it is dropped.
    claim: StdMutex<Option<Claim>>,
    pub offer_sdp: String,
    /// Virtual monitor mode: the monitor and what is shown of it.
    monitor: Option<MonitorSession>,
    /// What a restarted capture pipeline needs.
    track: Arc<TrackLocalStaticSample>,
    ev_tx: UnboundedSender<PipelineEvent>,
    ctl_slot: Arc<StdMutex<Option<Arc<pipeline::PipelineCtl>>>>,
}

/// The virtual monitor of a session and how it is currently used.
struct MonitorSession {
    mgr: Arc<VdManager>,
    state: StdMutex<MonState>,
}

struct MonState {
    mon: VMonitor,
    /// Size and DPI the monitor was configured with (the keyboard view is smaller).
    spec: MonitorSpec,
    /// Height of the streamed part; equal to the monitor height when the whole monitor shows.
    view_h: u32,
}

/// How a session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseKind {
    /// The tab was closed, the app exited or the stream failed: a virtual monitor goes now.
    Closed,
    /// The phone's connection dropped: a virtual monitor is kept for [`GRACE`] so the phone
    /// can attach again.
    PhoneGone,
}

/// The PC window the session streams, as reported to the phone (`window` message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowReport {
    pub width: i32,
    pub height: i32,
    pub fitted: bool,
    pub mode: StreamMode,
    /// Monitor mode: the DPI percentage Windows applied.
    pub dpi: Option<u32>,
}

impl From<winutil::Resized> for WindowReport {
    fn from(r: winutil::Resized) -> Self {
        Self { width: r.got.0, height: r.got.1, fitted: r.fitted, mode: StreamMode::Window, dpi: None }
    }
}

/// Everything `Session::open` needs.
pub struct OpenParams {
    pub id: String,
    pub app_id: String,
    pub hwnd: Hwnd,
    pub title: String,
    pub req: SizeRequest,
    pub claim: Claim,
    /// A virtual monitor the app window has already been moved onto, and its manager.
    pub monitor: Option<(Arc<VdManager>, VMonitor)>,
}

pub struct OpenedSession {
    pub session: Arc<Session>,
    pub window: WindowReport,
    /// Local ICE candidates, to be forwarded to the phone (after the offer was sent).
    pub ice_rx: UnboundedReceiver<RTCIceCandidateInit>,
    /// Window closed / errors.
    pub events_rx: UnboundedReceiver<PipelineEvent>,
}

fn h264_capability() -> RTCRtpCodecCapability {
    RTCRtpCodecCapability {
        mime_type: MIME_TYPE_H264.to_owned(),
        clock_rate: 90000,
        channels: 0,
        sdp_fmtp_line: H264_FMTP.to_owned(),
        rtcp_feedback: vec![
            RTCPFeedback { typ: "nack".to_owned(), parameter: String::new() },
            RTCPFeedback { typ: "nack".to_owned(), parameter: "pli".to_owned() },
            RTCPFeedback { typ: "ccm".to_owned(), parameter: "fir".to_owned() },
        ],
    }
}

pub fn new_session_id() -> String {
    crate::store::hex(&crate::store::random_bytes::<8>())
}

impl Session {
    /// Create the peer connection + capture pipeline for an already-found window (and, in
    /// monitor mode, the monitor it was moved onto).
    pub async fn open(p: OpenParams) -> Result<OpenedSession> {
        let OpenParams { id, app_id, hwnd, title, req, claim, monitor } = p;
        let plan = plan_sizes(&req);
        let (shared, window, source, monitor_session);
        if let Some((mgr, mon)) = monitor {
            // The window is already on the monitor, maximised. Stream the whole monitor.
            let (vw, vh) = clamp_video(mon.rect.w.max(2) as u32, mon.rect.h.max(2) as u32);
            let pid = winutil::window_pid(hwnd);
            shared = Shared::new(vw, vh, InputTarget::Monitor(MonitorTarget::new(hwnd, pid, mon.rect)));
            window = WindowReport {
                width: mon.rect.w,
                height: mon.rect.h,
                fitted: false,
                mode: StreamMode::Monitor,
                dpi: Some(mon.dpi_actual),
            };
            source = CaptureSource::Monitor { hmonitor: mon.hmonitor, watch: hwnd };
            let spec = MonitorSpec { w: mon.rect.w as u32, h: mon.rect.h as u32, dpi_wanted: mon.dpi_wanted };
            monitor_session = Some(MonitorSession {
                mgr,
                state: StdMutex::new(MonState { view_h: spec.h, spec, mon }),
            });
        } else {
            // Size the app window first so the first captured frame is already right.
            let (cw, ch) = client_size_for(plan.video_w, plan.video_h, plan.window_scale());
            let got = tokio::task::spawn_blocking(move || winutil::resize_client(hwnd, cw, ch)).await??;
            tracing::info!("window {hwnd:#x} client {}x{} (wanted {cw}x{ch}, fitted {})", got.got.0, got.got.1, got.fitted);
            window = WindowReport::from(got);
            shared = Shared::new(plan.video_w, plan.video_h, InputTarget::Window(hwnd));
            source = CaptureSource::Window(hwnd);
            monitor_session = None;
        }

        // --- WebRTC
        let mut media = MediaEngine::default();
        media.register_codec(
            RTCRtpCodecParameters { capability: h264_capability(), payload_type: 102, ..Default::default() },
            RTPCodecType::Video,
        )?;
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media)?;
        let mut se = SettingEngine::default();
        se.set_include_loopback_candidate(true);
        se.set_network_types(vec![NetworkType::Udp4]);
        if let Ok(eph) = EphemeralUDP::new(ICE_UDP_MIN, ICE_UDP_MAX) {
            se.set_udp_network(UDPNetwork::Ephemeral(eph));
        }
        se.set_ice_timeouts(
            Some(Duration::from_secs(6)),
            Some(Duration::from_secs(20)),
            Some(Duration::from_secs(2)),
        );
        let api = APIBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_setting_engine(se)
            .build();
        let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

        let track = Arc::new(TrackLocalStaticSample::new(h264_capability(), "video".to_owned(), "penlink".to_owned()));
        let transceiver = pc
            .add_transceiver_from_track(
                Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>,
                Some(RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Sendonly, send_encodings: vec![] }),
            )
            .await?;
        let sender = transceiver.sender().await;

        let dc = pc
            .create_data_channel("input", Some(RTCDataChannelInit { ordered: Some(true), ..Default::default() }))
            .await?;

        // --- input thread
        let (input_tx, input_rx) = std_mpsc::channel::<InputCmd>();
        let input_thread = {
            let shared = shared.clone();
            std::thread::Builder::new().name("penlink-input".into()).spawn(move || {
                let mut inj = Injector::new(shared);
                while let Ok(cmd) = input_rx.recv() {
                    match cmd {
                        InputCmd::Msg(m) => inj.handle(m),
                        InputCmd::Stop => break,
                    }
                }
                inj.release_all();
            })?
        };
        dc.on_open(Box::new(|| {
            tracing::info!("input data channel open");
            Box::pin(async {})
        }));
        {
            let tx = input_tx.clone();
            dc.on_message(Box::new(move |msg: DataChannelMessage| {
                let tx = tx.clone();
                Box::pin(async move {
                    match std::str::from_utf8(&msg.data).map(serde_json::from_str::<InputMsg>) {
                        Ok(Ok(m)) => {
                            tracing::trace!("input {m:?}");
                            let _ = tx.send(InputCmd::Msg(m));
                        }
                        Ok(Err(e)) => tracing::debug!("bad input message: {e}"),
                        Err(_) => tracing::debug!("non-utf8 input message"),
                    }
                })
            }));
        }

        // --- signalling plumbing
        let (ice_tx, ice_rx) = unbounded_channel::<RTCIceCandidateInit>();
        pc.on_ice_candidate(Box::new(move |c| {
            let tx = ice_tx.clone();
            Box::pin(async move {
                if let Some(c) = c {
                    if let Ok(init) = c.to_json() {
                        let _ = tx.send(init);
                    }
                }
            })
        }));
        let (ev_tx, events_rx) = unbounded_channel::<PipelineEvent>();
        let pipeline_ctl_slot: Arc<StdMutex<Option<Arc<pipeline::PipelineCtl>>>> = Arc::new(StdMutex::new(None));
        {
            let ev_tx = ev_tx.clone();
            let slot = pipeline_ctl_slot.clone();
            pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
                let ev_tx = ev_tx.clone();
                let slot = slot.clone();
                Box::pin(async move {
                    tracing::info!("peer connection state: {s}");
                    match s {
                        RTCPeerConnectionState::Connected => {
                            if let Some(ctl) = slot.lock().unwrap().as_ref() {
                                ctl.want_keyframe.store(true, Ordering::SeqCst);
                            }
                        }
                        RTCPeerConnectionState::Failed => {
                            let _ = ev_tx.send(PipelineEvent::Error("connection failed".into()));
                        }
                        _ => {}
                    }
                })
            }));
        }

        // RTCP reader: PLI / FIR -> keyframe. Must run for interceptors (NACK etc.) to work.
        {
            let slot = pipeline_ctl_slot.clone();
            tokio::spawn(async move {
                let mut last_idr = Instant::now() - Duration::from_secs(5);
                while let Ok((pkts, _)) = sender.read_rtcp().await {
                    for p in pkts {
                        let any = p.as_any();
                        if any.is::<PictureLossIndication>() || any.is::<FullIntraRequest>() {
                            if last_idr.elapsed() > Duration::from_millis(400) {
                                last_idr = Instant::now();
                                if let Some(ctl) = slot.lock().unwrap().as_ref() {
                                    ctl.want_keyframe.store(true, Ordering::SeqCst);
                                }
                            }
                        }
                    }
                }
            });
        }

        // --- offer
        let offer = pc.create_offer(None).await?;
        let offer_sdp = offer.sdp.clone();
        pc.set_local_description(offer).await?;

        // --- capture pipeline
        let handle = pipeline::spawn(source, shared.clone(), track.clone(), Handle::current(), ev_tx.clone());
        *pipeline_ctl_slot.lock().unwrap() = Some(handle.ctl.clone());

        let session = Arc::new(Session {
            id,
            app_id,
            hwnd,
            title,
            shared,
            pc,
            pipeline: StdMutex::new(Some(handle)),
            input_tx,
            input_thread: StdMutex::new(Some(input_thread)),
            remote_set: AtomicBool::new(false),
            pending_ice: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
            resize_lock: Mutex::new(()),
            claim: StdMutex::new(Some(claim)),
            offer_sdp,
            monitor: monitor_session,
            track,
            ev_tx,
            ctl_slot: pipeline_ctl_slot,
        });
        Ok(OpenedSession { session, window, ice_rx, events_rx })
    }

    pub async fn apply_answer(&self, sdp: String) -> Result<()> {
        let answer = RTCSessionDescription::answer(sdp)?;
        self.pc.set_remote_description(answer).await?;
        self.remote_set.store(true, Ordering::SeqCst);
        let pending: Vec<_> = std::mem::take(&mut *self.pending_ice.lock().await);
        for c in pending {
            if let Err(e) = self.pc.add_ice_candidate(c).await {
                tracing::debug!("buffered ice candidate rejected: {e}");
            }
        }
        Ok(())
    }

    pub async fn add_remote_ice(&self, init: RTCIceCandidateInit) {
        if init.candidate.trim().is_empty() {
            return; // end-of-candidates marker
        }
        if !self.remote_set.load(Ordering::SeqCst) {
            self.pending_ice.lock().await.push(init);
            return;
        }
        if let Err(e) = self.pc.add_ice_candidate(init).await {
            tracing::debug!("ice candidate rejected: {e}");
        }
    }

    pub fn request_keyframe(&self) {
        if let Some(p) = self.pipeline.lock().unwrap().as_ref() {
            p.ctl.want_keyframe.store(true, Ordering::SeqCst);
        }
    }

    pub async fn resize(&self, req: SizeRequest) -> Result<WindowReport> {
        let _one_at_a_time = self.resize_lock.lock().await;
        let plan = plan_sizes(&req);
        if self.monitor.is_some() {
            return self.resize_monitor(&plan).await;
        }
        let (cw, ch) = client_size_for(plan.video_w, plan.video_h, plan.window_scale());
        let hwnd = self.hwnd;
        let got = tokio::task::spawn_blocking(move || winutil::resize_client(hwnd, cw, ch)).await??;
        tracing::info!("resized window to client {}x{} (wanted {cw}x{ch}, fitted {})", got.got.0, got.got.1, got.fitted);
        self.shared.video_w.store(plan.video_w, Ordering::Relaxed);
        self.shared.video_h.store(plan.video_h, Ordering::Relaxed);
        self.request_keyframe();
        Ok(got.into())
    }

    /// Monitor mode. Same width and not taller than the monitor (the phone keyboard going up or
    /// down): only the streamed part and the window change. Anything else (rotation, quality)
    /// gives the monitor a new size.
    async fn resize_monitor(&self, plan: &crate::sizing::SizePlan) -> Result<WindowReport> {
        let ms = self.monitor.as_ref().expect("monitor mode");
        let (mon, spec, cur_view_h) = {
            let st = ms.state.lock().unwrap();
            (st.mon.clone(), st.spec, st.view_h)
        };
        let action = decide_resize(spec, plan);
        tracing::info!("resize {}x{} dpi {} -> {action:?} (monitor {}x{}, view {cur_view_h})", plan.video_w, plan.video_h, plan.dpi_pct, spec.w, spec.h);
        let hwnd = self.hwnd;
        let mgr = ms.mgr.clone();
        let (mon, spec, view_h) = match action {
            ResizeAction::Reconfigure { w, h } => {
                // The capture must not see the monitor disappear: stop it first.
                self.stop_pipeline().await;
                let dpi = plan.dpi_pct;
                let m2 = mon.clone();
                let new_mon = match tokio::task::spawn_blocking(move || -> Result<VMonitor> {
                    let m = mgr.reconfigure(&m2, w, h, dpi)?;
                    winutil::place_maximized_on(hwnd, m.rect)?;
                    Ok(m)
                })
                .await?
                {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = self.ev_tx.send(PipelineEvent::Error(format!("virtual monitor: {e}")));
                        return Err(e);
                    }
                };
                let spec = MonitorSpec { w: new_mon.rect.w as u32, h: new_mon.rect.h as u32, dpi_wanted: dpi };
                (new_mon, spec, spec.h)
            }
            ResizeAction::InPlace { view_h, dpi_changed } => {
                let mut mon = mon;
                let mut spec = spec;
                if dpi_changed {
                    let (m2, dpi) = (mon.clone(), plan.dpi_pct);
                    let mgr = mgr.clone();
                    mon = tokio::task::spawn_blocking(move || mgr.set_dpi(&m2, dpi)).await??;
                    spec.dpi_wanted = plan.dpi_pct;
                }
                let view_h = view_h.min(spec.h);
                if view_h != cur_view_h || dpi_changed {
                    let m2 = mon.clone();
                    let spec_h = spec.h;
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        if view_h >= spec_h {
                            winutil::place_maximized_on(hwnd, m2.rect)
                        } else {
                            winutil::place_view_on(hwnd, Rect::new(m2.rect.x, m2.rect.y, m2.rect.w, view_h as i32)).map(|_| ())
                        }
                    })
                    .await??;
                }
                (mon, spec, view_h)
            }
        };
        let monitor_rect = mon.rect;
        let view = Rect::new(monitor_rect.x, monitor_rect.y, monitor_rect.w, view_h as i32);
        if let InputTarget::Monitor(t) = &self.shared.target {
            t.set(monitor_rect, view);
        }
        let (vw, vh) = clamp_video(view.w.max(2) as u32, view.h.max(2) as u32);
        self.shared.video_w.store(vw, Ordering::Relaxed);
        self.shared.video_h.store(vh, Ordering::Relaxed);
        let hmonitor = mon.hmonitor;
        let report = WindowReport {
            width: view.w,
            height: view.h,
            fitted: false,
            mode: StreamMode::Monitor,
            dpi: Some(mon.dpi_actual),
        };
        *ms.state.lock().unwrap() = MonState { mon, spec, view_h };
        if matches!(action, ResizeAction::Reconfigure { .. }) {
            self.start_pipeline(CaptureSource::Monitor { hmonitor, watch: hwnd });
        }
        self.request_keyframe();
        Ok(report)
    }

    async fn stop_pipeline(&self) {
        let pipeline = self.pipeline.lock().unwrap().take();
        *self.ctl_slot.lock().unwrap() = None;
        let _ = tokio::task::spawn_blocking(move || drop(pipeline)).await;
    }

    fn start_pipeline(&self, source: CaptureSource) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        let handle = pipeline::spawn(source, self.shared.clone(), self.track.clone(), Handle::current(), self.ev_tx.clone());
        *self.ctl_slot.lock().unwrap() = Some(handle.ctl.clone());
        *self.pipeline.lock().unwrap() = Some(handle);
    }

    /// Stop everything. Does not touch the app itself. A virtual monitor is removed.
    pub async fn close(&self) {
        self.close_with(CloseKind::Closed).await;
    }

    /// Like [`close`](Self::close); `PhoneGone` keeps a virtual monitor for the grace period.
    pub async fn close_with(&self, kind: CloseKind) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.input_tx.send(InputCmd::Stop);
        let pipeline = self.pipeline.lock().unwrap().take();
        let input_thread = self.input_thread.lock().unwrap().take();
        let _ = tokio::task::spawn_blocking(move || {
            drop(pipeline); // joins the capture thread
            if let Some(t) = input_thread {
                let _ = t.join();
            }
        })
        .await;
        let _ = self.pc.close().await;
        if let Some(ms) = &self.monitor {
            let guid = ms.state.lock().unwrap().mon.guid;
            let mgr = ms.mgr.clone();
            let hwnd = self.hwnd;
            let _ = tokio::task::spawn_blocking(move || match kind {
                CloseKind::PhoneGone if winutil::is_window(hwnd) => mgr.orphan(&guid, std::time::Instant::now(), GRACE),
                _ => mgr.release(&guid),
            })
            .await;
        }
        // Capture and input threads are gone: the window may be opened again.
        self.claim.lock().unwrap().take();
    }

    /// Whether this session streams a virtual monitor.
    pub fn is_monitor(&self) -> bool {
        self.monitor.is_some()
    }
}

#[allow(dead_code)]
fn _assert_send() {
    fn is_send<T: Send>() {}
    is_send::<UnboundedSender<PipelineEvent>>();
    let _ = anyhow!("x");
}
