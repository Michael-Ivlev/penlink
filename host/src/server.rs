//! HTTP + WebSocket server (SPEC "Protocol").

use crate::apps::{self, AppInfo};
use crate::launcher::{self, Target};
use crate::netfilter;
use crate::notifications::{self, NotifEvent, Notifier, WinSource};
use crate::pipeline::PipelineEvent;
use crate::protocol::*;
use crate::session::{new_session_id, CloseKind, OpenParams, Session};
use crate::sizing::{plan_sizes, SizePlan, SizeRequest};
use crate::vdmanager::{VMonitor, VdManager};
use crate::store::{PairCheck, PairingCode, TokenStore};
use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::{oneshot, Mutex};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
const CATALOG_TTL: Duration = Duration::from_secs(30);
/// WebSocket keep-alive: ping interval, and how long the phone may stay silent (pongs count)
/// before it is treated as gone.
const PING_EVERY: Duration = Duration::from_secs(4);
const IDLE_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub bind: IpAddr,
    pub allow_lan: bool,
    /// Generate and print a new pairing code even if devices are already paired.
    pub force_pair: bool,
    /// Token store location; `None` = `%APPDATA%\Penlink\host.json`.
    pub store_path: Option<PathBuf>,
    pub host_name: Option<String>,
    /// Print the pairing code on the console.
    pub print_code: bool,
    /// Use the SudoVDA driver (one virtual monitor per opened app) when it is installed.
    pub virtual_monitors: bool,
    /// Forward Windows notifications to phones that connect to `/notify` (asks Windows for
    /// permission once). Off by default so tests never show a prompt; `main` turns it on.
    pub forward_notifications: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            allow_lan: false,
            force_pair: false,
            store_path: None,
            host_name: None,
            print_code: true,
            virtual_monitors: true,
            forward_notifications: false,
        }
    }
}

struct Catalog {
    apps: Vec<AppInfo>,
    at: Option<Instant>,
}

pub struct AppState {
    cfg: Config,
    host_name: String,
    store: Mutex<TokenStore>,
    pairing: Mutex<Option<PairingCode>>,
    catalog: Mutex<Catalog>,
    icons: Mutex<HashMap<String, Vec<u8>>>,
    /// Virtual monitors; `None` when the driver is not installed (window capture is used).
    vd: Option<Arc<VdManager>>,
    /// The session lists of the live WebSockets, so a phone that comes back after a network change
    /// can take over what its old (not yet noticed as dead) connection still holds.
    conns: std::sync::Mutex<Vec<std::sync::Weak<ConnSessions>>>,
    /// Windows notification forwarding; `None` when turned off.
    notifier: Option<Arc<Notifier>>,
}

impl AppState {
    pub async fn pairing_code(&self) -> Option<String> {
        let now = Instant::now();
        self.pairing.lock().await.as_ref().filter(|c| c.is_live(now)).map(|c| c.code().to_string())
    }

    async fn new_code(&self, announce: bool) -> String {
        let c = PairingCode::generate(Instant::now());
        let code = c.code().to_string();
        *self.pairing.lock().await = Some(c);
        if announce && self.cfg.print_code {
            print_code(&code);
        }
        code
    }

    async fn apps(&self, force: bool) -> Vec<AppInfo> {
        let mut cat = self.catalog.lock().await;
        let stale = cat.at.map(|t| t.elapsed() > CATALOG_TTL).unwrap_or(true);
        if force || stale {
            match tokio::task::spawn_blocking(apps::scan_apps).await {
                Ok(v) => {
                    tracing::debug!("scanned {} apps", v.len());
                    cat.apps = v;
                    cat.at = Some(Instant::now());
                }
                Err(e) => tracing::warn!("app scan failed: {e}"),
            }
        }
        cat.apps.clone()
    }

    async fn find_app(&self, id: &str) -> Option<AppInfo> {
        if let Some(a) = self.apps(false).await.into_iter().find(|a| a.id == id) {
            return Some(a);
        }
        self.apps(true).await.into_iter().find(|a| a.id == id)
    }
}

pub fn print_code(code: &str) {
    println!();
    println!("  ==========================================");
    println!("   Penlink pairing code:  {code}");
    println!("   valid for 5 minutes, enter it on the phone");
    println!("  ==========================================");
    println!();
}

pub fn host_name_from_env() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "penlink-host".to_string())
}

pub struct RunningServer {
    pub addr: SocketAddr,
    pub state: Arc<AppState>,
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<tokio::task::JoinHandle<()>>,
    reaper: Option<tokio::task::JoinHandle<()>>,
    notif_poller: Option<tokio::task::JoinHandle<()>>,
}

impl RunningServer {
    /// The currently valid pairing code, if any (also used by tests).
    pub async fn pairing_code(&self) -> Option<String> {
        self.state.pairing_code().await
    }

    pub async fn shutdown(mut self) {
        if let Some(p) = self.notif_poller.take() {
            p.abort();
        }
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(j) = self.join.take() {
            let _ = j.await;
        }
        self.remove_virtual_monitors().await;
    }

    /// Remove every virtual monitor now (their windows go back to the main monitor).
    pub async fn remove_virtual_monitors(&self) {
        if let Some(r) = self.reaper.as_ref() {
            r.abort();
        }
        if let Some(vd) = self.state.vd.clone() {
            let _ = tokio::task::spawn_blocking(move || vd.release_all()).await;
        }
    }

    /// Whether virtual monitors are available (the driver is installed and usable).
    pub fn has_virtual_monitors(&self) -> bool {
        self.state.vd.is_some()
    }

    /// Wait until the server task ends (it normally never does).
    pub async fn wait(&mut self) {
        if let Some(j) = self.join.as_mut() {
            let _ = j.await;
        }
    }
}

pub async fn start(cfg: Config) -> Result<RunningServer> {
    crate::winutil::init_process();
    let path = cfg.store_path.clone().unwrap_or_else(TokenStore::default_path);
    let store = TokenStore::load(&path);
    let need_code = cfg.force_pair || !store.has_devices();
    let host_name = cfg.host_name.clone().unwrap_or_else(host_name_from_env);
    let listener = TcpListener::bind(SocketAddr::new(cfg.bind, cfg.port))
        .await
        .with_context(|| format!("cannot listen on {}:{}", cfg.bind, cfg.port))?;
    let addr = listener.local_addr()?;
    let vd = if cfg.virtual_monitors { tokio::task::spawn_blocking(crate::vdisplay::start_manager).await.ok().flatten() } else { None };
    let notifier = cfg.forward_notifications.then(|| Notifier::new(Arc::new(WinSource::new())));
    let state = Arc::new(AppState {
        cfg,
        host_name,
        store: Mutex::new(store),
        pairing: Mutex::new(None),
        catalog: Mutex::new(Catalog { apps: Vec::new(), at: None }),
        icons: Mutex::new(HashMap::new()),
        vd: vd.clone(),
        conns: std::sync::Mutex::new(Vec::new()),
        notifier: notifier.clone(),
    });
    let notif_poller = notifier.map(|n| {
        // The permission prompt blocks, so it must not hold up the server.
        let n2 = n.clone();
        tokio::task::spawn_blocking(move || {
            n2.ensure_access_blocking();
        });
        n.spawn_poller()
    });
    // Monitors kept for a phone that never came back are removed after their grace period.
    let reaper = vd.map(|vd| {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let vd = vd.clone();
                let _ = tokio::task::spawn_blocking(move || vd.reap(Instant::now())).await;
            }
        })
    });
    if need_code {
        state.new_code(true).await;
    }

    let public = Router::new()
        .route("/info", get(info))
        .route("/pair", post(pair));
    let private = Router::new()
        .route("/apps", get(list_apps))
        .route("/apps/{id}/icon", get(app_icon))
        .route("/appicon", get(notif_app_icon))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_bearer));
    let ws = Router::new().route("/ws", get(ws_handler)).route("/notify", get(notify_handler));
    let app = Router::new()
        .merge(public)
        .merge(private)
        .merge(ws)
        .layer(middleware::from_fn_with_state(state.clone(), check_peer))
        .with_state(state.clone());

    let (tx, rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        let svc = app.into_make_service_with_connect_info::<SocketAddr>();
        let r = axum::serve(listener, svc)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
        if let Err(e) = r {
            tracing::error!("server error: {e}");
        }
    });
    tracing::info!("listening on {addr}");
    Ok(RunningServer { addr, state, shutdown: Some(tx), join: Some(join), reaper, notif_poller })
}

// ---------------------------------------------------------------------------------------
// middleware

async fn check_peer(
    State(st): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if !netfilter::is_allowed(peer.ip(), st.cfg.allow_lan) {
        tracing::warn!("rejected connection from {}", peer.ip());
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = v.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(rest.trim().to_string())
    } else {
        None
    }
}

async fn require_bearer(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let ok = match bearer(req.headers()) {
        Some(t) => st.store.lock().await.is_valid_token(&t),
        None => false,
    };
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(req).await
}

// ---------------------------------------------------------------------------------------
// handlers

async fn info(State(st): State<Arc<AppState>>) -> Json<InfoResponse> {
    Json(InfoResponse { host_name: st.host_name.clone(), version: VERSION.to_string() })
}

async fn pair(State(st): State<Arc<AppState>>, body: Result<Json<PairRequest>, axum::extract::rejection::JsonRejection>) -> Response {
    let Ok(Json(req)) = body else { return StatusCode::BAD_REQUEST.into_response() };
    let now = Instant::now();
    let verdict = {
        let mut p = st.pairing.lock().await;
        match p.as_mut() {
            None => PairCheck::Expired,
            Some(c) => {
                let v = c.check(&req.code, now);
                if v == PairCheck::Ok {
                    *p = None; // one-time use
                }
                v
            }
        }
    };
    match verdict {
        PairCheck::Ok => {
            let token = st.store.lock().await.add_device(&req.device_name);
            tracing::info!("paired device {:?}", req.device_name);
            Json(PairResponse { token, host_name: st.host_name.clone() }).into_response()
        }
        PairCheck::Wrong => StatusCode::FORBIDDEN.into_response(),
        PairCheck::Expired => {
            // A timed-out code is replaced (and shown on the host console) when nothing is
            // paired yet; a burned code stays burned until it times out.
            let needs_new = {
                let p = st.pairing.lock().await;
                p.as_ref().map(|c| c.timed_out(now)).unwrap_or(false)
            };
            if needs_new && !st.store.lock().await.has_devices() {
                st.new_code(true).await;
            }
            StatusCode::FORBIDDEN.into_response()
        }
    }
}

async fn list_apps(State(st): State<Arc<AppState>>) -> Json<Vec<AppEntry>> {
    Json(st.apps(false).await.iter().map(|a| a.entry()).collect())
}

async fn app_icon(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if let Some(png) = st.icons.lock().await.get(&id) {
        return png_response(png.clone());
    }
    let Some(app) = st.find_app(&id).await else { return StatusCode::NOT_FOUND.into_response() };
    let r = tokio::task::spawn_blocking(move || apps::app_icon_png(&app)).await;
    match r {
        Ok(Ok(png)) => {
            st.icons.lock().await.insert(id, png.clone());
            png_response(png)
        }
        Ok(Err(e)) => {
            tracing::debug!("no icon for {id}: {e}");
            StatusCode::NOT_FOUND.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn png_response(png: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "max-age=3600")], png).into_response()
}

#[derive(serde::Deserialize)]
struct AppIconQuery {
    #[serde(rename = "appId")]
    app_id: String,
}

/// Logo of the app behind a forwarded notification (as Windows gave it; usually PNG).
async fn notif_app_icon(State(st): State<Arc<AppState>>, Query(q): Query<AppIconQuery>) -> Response {
    let bytes = st.notifier.as_ref().and_then(|n| n.logo(&q.app_id));
    match bytes {
        Some(b) => {
            let ct = if b.starts_with(&[0x89, b'P', b'N', b'G']) {
                "image/png"
            } else if b.starts_with(&[0xFF, 0xD8]) {
                "image/jpeg"
            } else {
                "application/octet-stream"
            };
            ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "max-age=3600")], b).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn notify_handler(
    State(st): State<Arc<AppState>>,
    Query(q): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let token = q.token.or_else(|| bearer(&headers));
    let ok = match token {
        Some(t) => st.store.lock().await.is_valid_token(&t),
        None => false,
    };
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(notifier) = st.notifier.clone() else { return StatusCode::NOT_FOUND.into_response() };
    ws.on_upgrade(move |sock| notify_loop(sock, st, notifier))
}

/// One phone's notification socket: `ready`, then every added/removed notification; the phone
/// may send `dismissNotification`. It lives as long as the phone wants (pings every 4 s,
/// 12 s of silence = gone).
async fn notify_loop(socket: WebSocket, st: Arc<AppState>, notifier: Arc<Notifier>) {
    let (mut sink, mut stream) = socket.split();
    let mut sub = notifier.subscribe().await;
    let ready = NotifyServerMsg::Ready { ids: sub.ids.clone(), access: sub.access.wire().to_string() };
    if send_json(&mut sink, &ready).await.is_err() {
        return;
    }
    tracing::info!("phone listens for notifications ({} connected)", notifier.clients());
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut last_heard = Instant::now();
    loop {
        tokio::select! {
            ev = sub.events.recv() => {
                let msg = match ev {
                    Ok(NotifEvent::Added(n)) => {
                        let apps = st.apps(false).await;
                        NotifyServerMsg::Notification {
                            open_app_id: notifications::map_app(&apps, &n.app_id, &n.app_name),
                            id: n.id, app_id: n.app_id, app_name: n.app_name, title: n.title, body: n.body, time: n.time_ms,
                        }
                    }
                    Ok(NotifEvent::Removed(id)) => NotifyServerMsg::NotificationRemoved { id },
                    // Fell behind (Lagged) or closed: end the socket, the phone reconnects and resyncs.
                    Err(_) => break,
                };
                if send_json(&mut sink, &msg).await.is_err() { break; }
            }
            m = stream.next() => {
                match m {
                    Some(Ok(Message::Text(t))) => {
                        last_heard = Instant::now();
                        if let Ok(NotifyClientMsg::DismissNotification { id }) = serde_json::from_str(&t) {
                            notifier.dismiss(id).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => { last_heard = Instant::now(); }
                }
            }
            _ = ping.tick() => {
                if last_heard.elapsed() > IDLE_TIMEOUT { break; }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
    let _ = sink.close().await;
    tracing::info!("phone stopped listening for notifications");
}

async fn send_json<S>(sink: &mut S, msg: &NotifyServerMsg) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let s = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(s.into())).await.map_err(|_| ())
}

#[derive(serde::Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn ws_handler(
    State(st): State<Arc<AppState>>,
    Query(q): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let token = q.token.or_else(|| bearer(&headers));
    let ok = match token {
        Some(t) => st.store.lock().await.is_valid_token(&t),
        None => false,
    };
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |sock| ws_loop(sock, st))
}

/// Sessions of one WebSocket: `pending` ones were accepted (`opened` sent) but are still
/// launching their app; `ready` ones have a peer connection.
#[derive(Default)]
struct ConnSessions {
    ready: Mutex<HashMap<String, Arc<Session>>>,
    pending: Mutex<HashSet<String>>,
}

type Sessions = Arc<ConnSessions>;

async fn ws_loop(socket: WebSocket, st: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = unbounded_channel::<ServerMsg>();
    let writer = tokio::spawn(async move {
        // A ping every few seconds: a phone that vanished (network gone, no goodbye) answers
        // nothing and is noticed by the reader's idle timeout.
        let mut ping = tokio::time::interval(PING_EVERY);
        loop {
            tokio::select! {
                m = out_rx.recv() => {
                    let Some(m) = m else { break };
                    let Ok(s) = serde_json::to_string(&m) else { continue };
                    if sink.send(Message::Text(s.into())).await.is_err() {
                        break;
                    }
                }
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sink.close().await;
    });
    let sessions: Sessions = Arc::new(ConnSessions::default());
    st.conns.lock().unwrap().push(Arc::downgrade(&sessions));

    // How the phone left: a close frame = it meant to leave (tabs closed, app swiped away);
    // anything else = the link broke, the monitors get a short grace period.
    let mut goodbye = false;
    loop {
        let msg = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(_) => break,
            Err(_) => {
                tracing::info!("phone silent for {} s: treating the connection as dropped", IDLE_TIMEOUT.as_secs());
                break;
            }
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => {
                goodbye = true;
                break;
            }
            _ => continue,
        };
        let cm: ClientMsg = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let _ = out_tx.send(ServerMsg::Error { message: format!("bad message: {e}") });
                continue;
            }
        };
        match cm {
            ClientMsg::Open { app_id, width, height, scale, attach, quality, ui_scale } => {
                let req = SizeRequest::from_wire(width, height, scale, quality.as_deref(), ui_scale);
                let want_monitor = SizeRequest::wants_monitor(quality.as_deref(), ui_scale);
                tokio::spawn(do_open(st.clone(), out_tx.clone(), sessions.clone(), app_id, req, want_monitor, attach));
            }
            ClientMsg::Answer { session_id, sdp } => {
                if let Some(s) = get_session(&sessions, &session_id).await {
                    if let Err(e) = s.apply_answer(sdp).await {
                        let _ = out_tx.send(ServerMsg::Error { message: format!("answer rejected: {e}") });
                    }
                } else {
                    unknown_session(&out_tx, &session_id);
                }
            }
            ClientMsg::Ice { session_id, candidate, sdp_mid, sdp_m_line_index } => {
                if let Some(s) = get_session(&sessions, &session_id).await {
                    s.add_remote_ice(RTCIceCandidateInit {
                        candidate,
                        sdp_mid,
                        sdp_mline_index: sdp_m_line_index,
                        username_fragment: None,
                    })
                    .await;
                } else {
                    unknown_session(&out_tx, &session_id);
                }
            }
            ClientMsg::Resize { session_id, width, height, scale, quality, ui_scale } => {
                if let Some(s) = get_session(&sessions, &session_id).await {
                    let req = SizeRequest::from_wire(width, height, scale, quality.as_deref(), ui_scale);
                    let out_tx = out_tx.clone();
                    // A monitor change can take seconds: do not stall the other messages.
                    tokio::spawn(async move {
                        match s.resize(req).await {
                            Ok(w) => {
                                let _ = out_tx.send(window_msg(&s.id, &w));
                            }
                            Err(e) => {
                                let _ = out_tx.send(ServerMsg::Error { message: format!("resize failed: {e}") });
                            }
                        }
                    });
                } else {
                    unknown_session(&out_tx, &session_id);
                }
            }
            ClientMsg::Keyframe { session_id } => {
                if let Some(s) = get_session(&sessions, &session_id).await {
                    s.request_keyframe();
                }
            }
            ClientMsg::Close { session_id } => {
                let s = sessions.ready.lock().await.remove(&session_id);
                if let Some(s) = s {
                    // Not awaited here: the next messages must not wait for the monitor to go.
                    let out_tx = out_tx.clone();
                    tokio::spawn(async move {
                        s.close().await;
                        let _ = out_tx.send(ServerMsg::Closed { session_id, reason: "closed".into() });
                    });
                } else if sessions.pending.lock().await.remove(&session_id) {
                    // Still launching: do_open notices the missing entry and cleans up.
                    let _ = out_tx.send(ServerMsg::Closed { session_id, reason: "closed".into() });
                } else {
                    unknown_session(&out_tx, &session_id);
                }
            }
        }
    }

    // Phone went away: stop every session (apps stay open). After a goodbye the virtual
    // monitors go at once; after a broken link they stay for the (short) grace period so a
    // phone that is back within seconds can attach to them.
    sessions.pending.lock().await.clear();
    let all: Vec<Arc<Session>> = sessions.ready.lock().await.drain().map(|(_, s)| s).collect();
    let kind = if goodbye { CloseKind::Closed } else { CloseKind::PhoneGone };
    tracing::info!("phone left ({}), closing {} session(s)", if goodbye { "goodbye" } else { "link broke" }, all.len());
    let tasks: Vec<_> = all.into_iter().map(|s| tokio::spawn(async move { s.close_with(kind).await })).collect();
    for t in tasks {
        let _ = t.await;
    }
    drop(out_tx);
    let _ = writer.await;
}

fn unknown_session(out: &UnboundedSender<ServerMsg>, id: &str) {
    let _ = out.send(ServerMsg::Error { message: format!("unknown session {id}") });
}

async fn get_session(sessions: &Sessions, id: &str) -> Option<Arc<Session>> {
    sessions.ready.lock().await.get(id).cloned()
}

async fn resolve_target(st: &AppState, app_id: &str) -> Option<Target> {
    if app_id.starts_with("exe:") {
        return Target::from_exe_id(app_id);
    }
    st.find_app(app_id).await.map(|a| Target::from_app(&a))
}

fn window_msg(session_id: &str, w: &crate::session::WindowReport) -> ServerMsg {
    ServerMsg::Window { session_id: session_id.to_string(), width: w.width, height: w.height, fitted: w.fitted, mode: w.mode, dpi: w.dpi }
}

/// A new virtual monitor for `plan`, or `None` (with the reason logged) so the caller falls
/// back to window capture.
async fn create_monitor(vd: &Arc<VdManager>, plan: &SizePlan) -> Option<VMonitor> {
    let (vd, w, h, dpi) = (vd.clone(), plan.video_w, plan.video_h, plan.dpi_pct);
    match tokio::task::spawn_blocking(move || vd.create(w, h, dpi)).await {
        Ok(Ok(m)) => {
            tracing::info!(
                "virtual monitor {} {}x{} at {},{} dpi {}% (asked {}%)",
                m.gdi_name, m.rect.w, m.rect.h, m.rect.x, m.rect.y, m.dpi_actual, m.dpi_wanted
            );
            Some(m)
        }
        Ok(Err(e)) => {
            tracing::warn!("virtual monitor failed, using window capture: {e:#}");
            None
        }
        Err(e) => {
            tracing::warn!("virtual monitor task failed: {e}");
            None
        }
    }
}

/// A phone that reattaches takes over the monitor kept for it (resized if its screen is
/// different now), or gets a new one.
async fn adopt_or_create(vd: &Arc<VdManager>, hwnd: isize, plan: &SizePlan) -> Option<VMonitor> {
    let (vd2, plan2) = (vd.clone(), *plan);
    let adopted = tokio::task::spawn_blocking(move || -> Option<VMonitor> {
        let m = vd2.adopt(hwnd)?;
        let fitted = if (m.rect.w, m.rect.h) != (plan2.video_w as i32, plan2.video_h as i32) {
            vd2.reconfigure(&m, plan2.video_w, plan2.video_h, plan2.dpi_pct)
        } else if m.dpi_wanted != plan2.dpi_pct {
            vd2.set_dpi(&m, plan2.dpi_pct)
        } else {
            Ok(m)
        };
        match fitted {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!("kept virtual monitor could not be reused: {e:#}");
                None
            }
        }
    })
    .await
    .ok()
    .flatten();
    match adopted {
        Some(m) => {
            tracing::info!("phone is back: reusing virtual monitor {}", m.gdi_name);
            Some(m)
        }
        None => create_monitor(vd, plan).await,
    }
}

async fn release_monitor(vd: &Arc<VdManager>, m: &VMonitor) {
    let (vd, guid) = (vd.clone(), m.guid);
    let _ = tokio::task::spawn_blocking(move || vd.release(&guid)).await;
}

#[allow(clippy::too_many_arguments)]
async fn do_open(
    st: Arc<AppState>,
    out: UnboundedSender<ServerMsg>,
    sessions: Sessions,
    app_id: String,
    req: SizeRequest,
    want_monitor: bool,
    attach: bool,
) {
    // Accept first: the phone gets a session id (and a pending tab) before anything slow
    // happens, so every later `closed` refers to a session it knows.
    let session_id = new_session_id();
    let _ = out.send(ServerMsg::Opened { session_id: session_id.clone(), app_id: app_id.clone(), title: String::new() });
    sessions.pending.lock().await.insert(session_id.clone());
    let fail = |reason: String| {
        let _ = out.send(ServerMsg::Closed { session_id: session_id.clone(), reason });
    };
    let (width, height) = (req.width, req.height);
    if !(64..=8192).contains(&width) || !(64..=8192).contains(&height) {
        sessions.pending.lock().await.remove(&session_id);
        fail(format!("error: bad size {width}x{height}"));
        return;
    }
    let Some(target) = resolve_target(&st, &app_id).await else {
        sessions.pending.lock().await.remove(&session_id);
        fail(format!("error: unknown app {app_id}"));
        return;
    };
    let plan = plan_sizes(&req);
    let vd = if want_monitor { st.vd.clone() } else { None };
    tracing::info!(
        "open {:?} ({app_id}) {width}x{height} {} ui {}% monitor={} -> session {session_id}",
        target.name,
        req.quality.as_str(),
        req.ui_scale,
        vd.is_some()
    );

    // A re-attaching phone has lost its old connection, which the host may not know yet (a silent
    // network change): end that connection's session for this app now, as if it had dropped, so
    // its monitor is kept for a moment and taken over here instead of waiting for the timeout.
    if attach {
        let others: Vec<Sessions> = {
            let mut l = st.conns.lock().unwrap();
            l.retain(|w| w.strong_count() > 0);
            l.iter().filter_map(|w| w.upgrade()).filter(|c| !Arc::ptr_eq(c, &sessions)).collect()
        };
        for c in others {
            let stale: Vec<Arc<Session>> = {
                let mut ready = c.ready.lock().await;
                let ids: Vec<String> = ready.iter().filter(|(_, s)| s.app_id == app_id).map(|(k, _)| k.clone()).collect();
                ids.iter().filter_map(|k| ready.remove(k)).collect()
            };
            for s in stale {
                tracing::info!("session {} belongs to a connection the phone left: handing its app to the new one", s.id);
                s.close_with(CloseKind::PhoneGone).await;
            }
        }
    }

    // A new app gets its monitor first, so the window can be moved there as soon as it
    // appears. A reattach looks for the window first (a kept monitor is found through it).
    let mut monitor: Option<VMonitor> = None;
    if let (Some(vd), false) = (&vd, attach) {
        monitor = create_monitor(vd, &plan).await;
    }

    let found = tokio::task::spawn_blocking(move || launcher::launch_and_find(&target, LAUNCH_TIMEOUT, attach)).await;
    let found = match found {
        Ok(Ok(f)) => f,
        other => {
            let msg = match other {
                Ok(Err(e)) => {
                    tracing::warn!("open failed: {e}");
                    // A reattach that finds no window means the app is gone: not an error.
                    if attach && e.to_string() == "app exited" { "app exited".to_string() } else { format!("error: {e}") }
                }
                Err(e) => format!("error: {e}"),
                Ok(Ok(_)) => unreachable!(),
            };
            if let (Some(vd), Some(m)) = (&vd, &monitor) {
                release_monitor(vd, m).await;
            }
            if sessions.pending.lock().await.remove(&session_id) {
                fail(msg);
            }
            return;
        }
    };
    if !sessions.pending.lock().await.contains(&session_id) {
        tracing::info!("session {session_id} was closed while launching");
        if let (Some(vd), Some(m)) = (&vd, &monitor) {
            release_monitor(vd, m).await;
        }
        return; // phone closed the tab (or went away); the app stays open
    }
    if !found.title.is_empty() {
        let _ = out.send(ServerMsg::Title { session_id: session_id.clone(), title: found.title.clone() });
    }

    // Move the window onto the monitor and maximise it; any failure falls back to window capture.
    if let Some(vd) = &vd {
        if monitor.is_none() && attach {
            monitor = adopt_or_create(vd, found.hwnd, &plan).await;
        }
        if let Some(m) = monitor.clone() {
            let (hwnd, rect) = (found.hwnd, m.rect);
            vd.bind(&m.guid, hwnd, &app_id);
            let placed = tokio::task::spawn_blocking(move || crate::winutil::place_maximized_on(hwnd, rect)).await;
            if !matches!(placed, Ok(Ok(()))) {
                tracing::warn!("could not move the window onto the virtual monitor ({placed:?}); using window capture");
                release_monitor(vd, &m).await;
                monitor = None;
            }
        }
    }

    let opened = match Session::open(OpenParams {
        id: session_id.clone(),
        app_id: app_id.clone(),
        hwnd: found.hwnd,
        title: found.title.clone(),
        req,
        claim: found.claim,
        monitor: vd.clone().zip(monitor.clone()),
    })
    .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("session setup failed: {e:#}");
            if let (Some(vd), Some(m)) = (&vd, &monitor) {
                release_monitor(vd, m).await;
            }
            if sessions.pending.lock().await.remove(&session_id) {
                fail(format!("error: {e}"));
            }
            return;
        }
    };
    let session = opened.session.clone();
    if !sessions.pending.lock().await.remove(&session_id) {
        session.close().await; // closed during setup
        return;
    }
    sessions.ready.lock().await.insert(session_id.clone(), session.clone());

    let _ = out.send(window_msg(&session_id, &opened.window));
    let _ = out.send(ServerMsg::Offer { session_id: session_id.clone(), sdp: session.offer_sdp.clone() });

    // Local ICE candidates (trickle) - only after the offer went out.
    let mut ice_rx = opened.ice_rx;
    {
        let out = out.clone();
        let sid = session_id.clone();
        tokio::spawn(async move {
            while let Some(c) = ice_rx.recv().await {
                if out
                    .send(ServerMsg::Ice {
                        session_id: sid.clone(),
                        candidate: c.candidate,
                        sdp_mid: c.sdp_mid,
                        sdp_m_line_index: c.sdp_mline_index,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    // Window closed / pipeline error.
    let mut events_rx = opened.events_rx;
    tokio::spawn(async move {
        while let Some(ev) = events_rx.recv().await {
            let reason = match ev {
                PipelineEvent::WindowClosed => "app exited".to_string(),
                PipelineEvent::Error(e) => format!("error: {e}"),
                PipelineEvent::Window(w) => {
                    if sessions.ready.lock().await.contains_key(&session_id) {
                        let _ = out.send(window_msg(&session_id, &w));
                    }
                    continue;
                }
            };
            let removed = sessions.ready.lock().await.remove(&session_id);
            if removed.is_some() {
                session.close().await;
                let _ = out.send(ServerMsg::Closed { session_id, reason });
            }
            break;
        }
    });
}
