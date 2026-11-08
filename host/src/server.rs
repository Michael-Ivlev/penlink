//! HTTP + WebSocket server (SPEC "Protocol").

use crate::apps::{self, AppInfo};
use crate::launcher::{self, Target};
use crate::netfilter;
use crate::pipeline::PipelineEvent;
use crate::protocol::*;
use crate::session::{new_session_id, Session};
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
}

impl RunningServer {
    /// The currently valid pairing code, if any (also used by tests).
    pub async fn pairing_code(&self) -> Option<String> {
        self.state.pairing_code().await
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(j) = self.join.take() {
            let _ = j.await;
        }
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
    let state = Arc::new(AppState {
        cfg,
        host_name,
        store: Mutex::new(store),
        pairing: Mutex::new(None),
        catalog: Mutex::new(Catalog { apps: Vec::new(), at: None }),
        icons: Mutex::new(HashMap::new()),
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
        .route_layer(middleware::from_fn_with_state(state.clone(), require_bearer));
    let ws = Router::new().route("/ws", get(ws_handler));
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
    Ok(RunningServer { addr, state, shutdown: Some(tx), join: Some(join) })
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
        while let Some(m) = out_rx.recv().await {
            let Ok(s) = serde_json::to_string(&m) else { continue };
            if sink.send(Message::Text(s.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let sessions: Sessions = Arc::new(ConnSessions::default());

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
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
            ClientMsg::Open { app_id, width, height, scale, attach } => {
                tokio::spawn(do_open(st.clone(), out_tx.clone(), sessions.clone(), app_id, width, height, scale, attach));
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
            ClientMsg::Resize { session_id, width, height, scale } => {
                if let Some(s) = get_session(&sessions, &session_id).await {
                    match s.resize(width, height, scale).await {
                        Ok(w) => {
                            let _ = out_tx.send(ServerMsg::Window {
                                session_id: s.id.clone(),
                                width: w.width,
                                height: w.height,
                                fitted: w.fitted,
                            });
                        }
                        Err(e) => {
                            let _ = out_tx.send(ServerMsg::Error { message: format!("resize failed: {e}") });
                        }
                    }
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
                    s.close().await;
                    let _ = out_tx.send(ServerMsg::Closed { session_id, reason: "closed".into() });
                } else if sessions.pending.lock().await.remove(&session_id) {
                    // Still launching: do_open notices the missing entry and cleans up.
                    let _ = out_tx.send(ServerMsg::Closed { session_id, reason: "closed".into() });
                } else {
                    unknown_session(&out_tx, &session_id);
                }
            }
        }
    }

    // Phone went away: stop every session (apps stay open).
    sessions.pending.lock().await.clear();
    let all: Vec<Arc<Session>> = sessions.ready.lock().await.drain().map(|(_, s)| s).collect();
    for s in all {
        s.close().await;
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

async fn do_open(
    st: Arc<AppState>,
    out: UnboundedSender<ServerMsg>,
    sessions: Sessions,
    app_id: String,
    width: u32,
    height: u32,
    scale: f64,
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
    tracing::info!("open {:?} ({app_id}) {width}x{height} @{scale} -> session {session_id}", target.name);

    let found = tokio::task::spawn_blocking(move || launcher::launch_and_find(&target, LAUNCH_TIMEOUT, attach)).await;
    let found = match found {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => {
            tracing::warn!("open failed: {e}");
            if sessions.pending.lock().await.remove(&session_id) {
                // A reattach that finds no window means the app is gone: not an error.
                fail(if attach && e.to_string() == "app exited" { "app exited".to_string() } else { format!("error: {e}") });
            }
            return;
        }
        Err(e) => {
            if sessions.pending.lock().await.remove(&session_id) {
                fail(format!("error: {e}"));
            }
            return;
        }
    };
    if !sessions.pending.lock().await.contains(&session_id) {
        tracing::info!("session {session_id} was closed while launching");
        return; // phone closed the tab (or went away); the app stays open
    }
    if !found.title.is_empty() {
        let _ = out.send(ServerMsg::Title { session_id: session_id.clone(), title: found.title.clone() });
    }

    let opened = match Session::open(session_id.clone(), app_id.clone(), found.hwnd, found.title.clone(), width, height, scale, found.claim).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("session setup failed: {e:#}");
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

    let w = opened.window;
    let _ = out.send(ServerMsg::Window { session_id: session_id.clone(), width: w.width, height: w.height, fitted: w.fitted });
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
        if let Some(ev) = events_rx.recv().await {
            let reason = match ev {
                PipelineEvent::WindowClosed => "app exited".to_string(),
                PipelineEvent::Error(e) => format!("error: {e}"),
            };
            let removed = sessions.ready.lock().await.remove(&session_id);
            if removed.is_some() {
                session.close().await;
                let _ = out.send(ServerMsg::Closed { session_id, reason });
            }
        }
    });
}
