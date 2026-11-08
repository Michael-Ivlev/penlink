//! Stress test through the real server: starts `penlink-host.exe` as a child process (own token
//! store, own crash-report directory) and drives it over HTTP/WebSocket/WebRTC like phones do.
//! Several workers each loop: open a testpad session, receive video, send input over the data
//! channel, resize / rotate, then end it in one of several rude ways (close, kill the app, drop
//! the WebSocket, close while still launching). Rounds cycle through encoder thread counts.
//! If the host exits unexpectedly it is recorded (exit code, crash report) and restarted.
//!
//! Only `testpad` copies named `plstress-tp<N>.exe` (made in the run directory) are ever opened
//! or killed; nothing else on the PC is touched.
//!
//!   cargo build --release --examples && cargo build --release
//!   cargo run --release --example stress -- --secs 300 --workers 4 --round-secs 60 --threads 4,2,1,3
//!       [--host-exe target\release\penlink-host.exe] [--testpad target\release\examples\testpad.exe]
//!       [--dir <run dir>] [--share-exe]   (workers 0 and 1 use the same testpad copy)

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::ice::network_type::NetworkType;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
}

#[derive(Default)]
struct Stats {
    cycles: AtomicU64,
    opened: AtomicU64,
    offers: AtomicU64,
    first_rtp: AtomicU64,
    no_rtp: AtomicU64,
    rtp_frames: AtomicU64,
    input_msgs: AtomicU64,
    resizes: AtomicU64,
    kills: AtomicU64,
    kill_detected: AtomicU64,
    ws_drops: AtomicU64,
    early_closes: AtomicU64,
    conn_errors: AtomicU64,
    reasons: Mutex<BTreeMap<String, u64>>,
    host_crashes: Mutex<Vec<String>>,
}

impl Stats {
    fn reason(&self, r: &str) {
        let key: String = r.chars().take(70).collect();
        *self.reasons.lock().unwrap().entry(key).or_default() += 1;
    }
}

struct Args {
    secs: u64,
    workers: usize,
    round_secs: u64,
    threads: Vec<u16>,
    host_exe: PathBuf,
    testpad: PathBuf,
    dir: PathBuf,
    share_exe: bool,
    seed: u64,
}

fn parse_args() -> Args {
    let mut a = Args {
        secs: 300,
        workers: 4,
        round_secs: 60,
        threads: vec![4, 2, 1, 3],
        host_exe: PathBuf::from(r"target\release\penlink-host.exe"),
        testpad: PathBuf::from(r"target\release\examples\testpad.exe"),
        dir: std::env::temp_dir().join(format!("penlink-stress-{}", std::process::id())),
        share_exe: false,
        seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 | 1,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--secs" => a.secs = it.next().unwrap().parse().unwrap(),
            "--workers" => a.workers = it.next().unwrap().parse().unwrap(),
            "--round-secs" => a.round_secs = it.next().unwrap().parse().unwrap(),
            "--threads" => a.threads = it.next().unwrap().split(',').map(|s| s.parse().unwrap()).collect(),
            "--host-exe" => a.host_exe = it.next().unwrap().into(),
            "--testpad" => a.testpad = it.next().unwrap().into(),
            "--dir" => a.dir = it.next().unwrap().into(),
            "--share-exe" => a.share_exe = true,
            "--seed" => a.seed = it.next().unwrap().parse().unwrap(),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

async fn http(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> Result<(u16, String)> {
    let mut s = TcpStream::connect(addr).await?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).await?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("bad http"))?;
    Ok((status, text.split("\r\n\r\n").nth(1).unwrap_or("").to_string()))
}

fn exe_name(slot: usize) -> String {
    format!("plstress-tp{slot}.exe")
}

/// Kill the testpad copy of one slot (exact image name, so nothing else can match).
fn kill_testpad(slot: usize) -> bool {
    let out = Command::new("taskkill").args(["/F", "/IM", &exe_name(slot)]).output();
    matches!(out, Ok(o) if o.status.success())
}

fn kill_all_testpads(slots: usize) {
    for s in 0..slots {
        kill_testpad(s);
    }
}

struct Round {
    addr: SocketAddr,
    token: String,
    dead: AtomicBool,
}

struct Ctx {
    stats: Stats,
    dir: PathBuf,
    share_exe: bool,
}

const SIZES: &[(u32, u32)] = &[
    (1440, 3120),
    (3120, 1440),
    (720, 1560),
    (1560, 720),
    (1080, 2340),
    (2340, 1080),
    (1001, 777),
    (777, 1001),
    (200, 400),
    (64, 64),
    (3840, 2160),
    (2160, 3840),
    (8192, 1000),
    (1440, 2400),
    (2560, 1080),
];
const SCALES: &[f64] = &[1.0, 1.5, 2.0, 2.5, 3.0, 0.5, 4.0];
const TEXTS: &[&str] = &["hello world\n", "abc", "héllo wörld ✓", "line1\nline2\n\tindent", "{ } [ ] ( ) < > ; : ' \"", "日本語テキスト 😀"];
// letters, digits, arrows, backspace, enter, shift, ctrl, home/end (no Win/Alt/F-keys)
const VKS: &[u16] = &[0x41, 0x42, 0x43, 0x5A, 0x30, 0x31, 0x25, 0x26, 0x27, 0x28, 0x08, 0x0D, 0x10, 0x11, 0x24, 0x23, 0x20];

fn input_msg(rng: &mut Rng) -> Value {
    let (x, y) = (rng.f(), rng.f());
    match rng.below(10) {
        0 => json!({"t":"text","s": *rng.pick(TEXTS)}),
        1 => json!({"t":"key","a":"down","vk": *rng.pick(VKS)}),
        2 => json!({"t":"key","a":"up","vk": *rng.pick(VKS)}),
        3 => json!({"t":"mouse","a": *rng.pick(&["move","down","up"]),"x":x,"y":y,"btn": *rng.pick(&["left","right","middle"])}),
        4 => json!({"t":"mrel","dx": rng.f()*40.0-20.0,"dy": rng.f()*40.0-20.0}),
        5 => json!({"t":"wheel","dx": 0,"dy": (rng.below(5) as i64 - 2) * 120}),
        6 => json!({"t":"pen","a": *rng.pick(&["hover","down","move","up","leave"]),"x":x,"y":y,"p":rng.f(),"tx":rng.below(60) as i64-30,"ty":rng.below(60) as i64-30,"b":rng.below(2),"e":rng.below(8)==0}),
        7 => json!({"t":"touch","a": *rng.pick(&["down","move","up","cancel"]),"id": rng.below(3),"x":x,"y":y}),
        8 => json!({"t":"mouse","a":"move","x":x,"y":y,"btn":"left"}),
        _ => json!({"t":"text","s": format!("{:04} the quick brown fox\n", rng.below(10000))}),
    }
}

#[derive(Clone, Copy, Debug)]
enum End {
    Close,
    KillApp,
    DropWs,
    ResizeStormThenClose,
    CloseWhilePending,
}

async fn run_cycle(ctx: &Arc<Ctx>, round: &Arc<Round>, worker: usize, slot: usize, rng: &mut Rng) -> Result<()> {
    let st = &ctx.stats;
    st.cycles.fetch_add(1, Ordering::Relaxed);
    let log = ctx.dir.join(format!("testpad-{slot}.log"));
    let app_id = format!("exe:{} {}", ctx.dir.join(exe_name(slot)).display(), log.display());
    let &(w, h) = rng.pick(SIZES);
    let &scale = rng.pick(SCALES);

    // --- peer connection (answerer)
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media)?;
    let mut se = SettingEngine::default();
    se.set_include_loopback_candidate(true);
    se.set_network_types(vec![NetworkType::Udp4]);
    let api = APIBuilder::new().with_media_engine(media).with_interceptor_registry(registry).with_setting_engine(se).build();
    let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let session_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    {
        let tx = out_tx.clone();
        let sid = session_id.clone();
        pc.on_ice_candidate(Box::new(move |c| {
            let tx = tx.clone();
            let sid = sid.clone();
            Box::pin(async move {
                if let (Some(c), Some(sid)) = (c, sid.lock().unwrap().clone()) {
                    if let Ok(j) = c.to_json() {
                        let _ = tx.send(json!({"type":"ice","sessionId":sid,"candidate":j.candidate,"sdpMid":j.sdp_mid,"sdpMLineIndex":j.sdp_mline_index}));
                    }
                }
            })
        }));
    }
    let frames = Arc::new(AtomicU64::new(0));
    {
        let frames = frames.clone();
        let ctx = ctx.clone();
        pc.on_track(Box::new(move |track, _rx, _tr| {
            let frames = frames.clone();
            let ctx = ctx.clone();
            Box::pin(async move {
                tokio::spawn(async move {
                    while let Ok((pkt, _)) = track.read_rtp().await {
                        if pkt.header.marker {
                            frames.fetch_add(1, Ordering::Relaxed);
                            ctx.stats.rtp_frames.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            })
        }));
    }
    let dc_slot: Arc<Mutex<Option<Arc<webrtc::data_channel::RTCDataChannel>>>> = Arc::new(Mutex::new(None));
    {
        let slot = dc_slot.clone();
        pc.on_data_channel(Box::new(move |dc| {
            let slot = slot.clone();
            Box::pin(async move {
                *slot.lock().unwrap() = Some(dc);
            })
        }));
    }

    // --- websocket
    let url = format!("ws://{}/ws?token={}", round.addr, round.token);
    let (ws, _) = match tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(url)).await {
        Ok(Ok(v)) => v,
        _ => {
            st.conn_errors.fetch_add(1, Ordering::Relaxed);
            let _ = pc.close().await;
            return Err(anyhow!("websocket connect failed"));
        }
    };
    let (mut ws_tx, mut ws_rx) = ws.split();
    let writer = tokio::spawn(async move {
        while let Some(v) = out_rx.recv().await {
            if ws_tx.send(Message::Text(v.to_string().into())).await.is_err() {
                break;
            }
        }
        // dropping ws_tx without a close frame = abrupt drop
        drop(ws_tx);
    });

    let got_opened = Arc::new(AtomicBool::new(false));
    let got_offer = Arc::new(AtomicBool::new(false));
    let closed_reason: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let reader = {
        let pc = pc.clone();
        let sid = session_id.clone();
        let tx = out_tx.clone();
        let (go, gf, cr) = (got_opened.clone(), got_offer.clone(), closed_reason.clone());
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let mut pending: Vec<RTCIceCandidateInit> = Vec::new();
            let mut remote_set = false;
            while let Some(Ok(m)) = ws_rx.next().await {
                let Message::Text(t) = m else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                match v["type"].as_str().unwrap_or("") {
                    "opened" => {
                        *sid.lock().unwrap() = v["sessionId"].as_str().map(|s| s.to_string());
                        go.store(true, Ordering::SeqCst);
                        ctx.stats.opened.fetch_add(1, Ordering::Relaxed);
                    }
                    "offer" => {
                        let Ok(offer) = RTCSessionDescription::offer(v["sdp"].as_str().unwrap_or("").to_string()) else { continue };
                        if pc.set_remote_description(offer).await.is_err() {
                            continue;
                        }
                        let Ok(ans) = pc.create_answer(None).await else { continue };
                        let sdp = ans.sdp.clone();
                        if pc.set_local_description(ans).await.is_err() {
                            continue;
                        }
                        let sid = sid.lock().unwrap().clone().unwrap_or_default();
                        let _ = tx.send(json!({"type":"answer","sessionId":sid,"sdp":sdp}));
                        remote_set = true;
                        gf.store(true, Ordering::SeqCst);
                        ctx.stats.offers.fetch_add(1, Ordering::Relaxed);
                        for c in pending.drain(..) {
                            let _ = pc.add_ice_candidate(c).await;
                        }
                    }
                    "ice" => {
                        let c = RTCIceCandidateInit {
                            candidate: v["candidate"].as_str().unwrap_or("").to_string(),
                            sdp_mid: v["sdpMid"].as_str().map(|s| s.to_string()),
                            sdp_mline_index: v["sdpMLineIndex"].as_u64().map(|n| n as u16),
                            username_fragment: None,
                        };
                        if remote_set {
                            let _ = pc.add_ice_candidate(c).await;
                        } else {
                            pending.push(c);
                        }
                    }
                    "closed" => {
                        let r = v["reason"].as_str().unwrap_or("?").to_string();
                        ctx.stats.reason(&r);
                        *cr.lock().unwrap() = Some(r);
                    }
                    "window" | "title" | "error" => {
                        if v["type"] == "error" {
                            ctx.stats.reason(&format!("error msg: {}", v["message"].as_str().unwrap_or("?")));
                        }
                    }
                    _ => {}
                }
            }
        })
    };

    let end = match rng.below(100) {
        0..=29 => End::Close,
        30..=54 => End::KillApp,
        55..=74 => End::DropWs,
        75..=89 => End::ResizeStormThenClose,
        _ => End::CloseWhilePending,
    };
    let _ = out_tx.send(json!({"type":"open","appId":app_id,"width":w,"height":h,"scale":scale}));

    let finish = |pc: Arc<webrtc::peer_connection::RTCPeerConnection>, reader: tokio::task::JoinHandle<()>, writer: tokio::task::JoinHandle<()>| async move {
        let _ = tokio::time::timeout(Duration::from_secs(5), pc.close()).await;
        reader.abort();
        writer.abort();
    };

    // early close: right after the host accepted the open (maybe still launching)
    if matches!(end, End::CloseWhilePending) {
        let t = Instant::now();
        while !got_opened.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(rng.range(0, 1500))).await;
        st.early_closes.fetch_add(1, Ordering::Relaxed);
        let sid = session_id.lock().unwrap().clone();
        if let Some(sid) = sid {
            let _ = out_tx.send(json!({"type":"close","sessionId":sid}));
        }
        tokio::time::sleep(Duration::from_millis(rng.range(300, 1500))).await;
        finish(pc, reader, writer).await;
        return Ok(());
    }

    // wait for the offer
    let t = Instant::now();
    while !got_offer.load(Ordering::SeqCst) {
        if closed_reason.lock().unwrap().is_some() || round.dead.load(Ordering::SeqCst) || t.elapsed() > Duration::from_secs(25) {
            finish(pc, reader, writer).await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // first RTP
    let t = Instant::now();
    while frames.load(Ordering::Relaxed) == 0 {
        if closed_reason.lock().unwrap().is_some() || round.dead.load(Ordering::SeqCst) {
            finish(pc, reader, writer).await;
            return Ok(());
        }
        if t.elapsed() > Duration::from_secs(15) {
            st.no_rtp.fetch_add(1, Ordering::Relaxed);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if frames.load(Ordering::Relaxed) > 0 {
        st.first_rtp.fetch_add(1, Ordering::Relaxed);
    }

    // --- active phase
    let sid = session_id.lock().unwrap().clone().unwrap_or_default();
    let active = Duration::from_millis(rng.range(1500, 6000));
    let t = Instant::now();
    let mut next_resize = Instant::now() + Duration::from_millis(rng.range(200, 1200));
    let storm = matches!(end, End::ResizeStormThenClose);
    while t.elapsed() < active {
        if closed_reason.lock().unwrap().is_some() || round.dead.load(Ordering::SeqCst) {
            break;
        }
        let dc = dc_slot.lock().unwrap().clone();
        if let Some(dc) = dc {
            for _ in 0..rng.range(1, 6) {
                let m = input_msg(rng);
                if dc.send_text(m.to_string()).await.is_ok() {
                    st.input_msgs.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        if Instant::now() >= next_resize {
            let &(rw, rh) = rng.pick(SIZES);
            let &rs = rng.pick(SCALES);
            let _ = out_tx.send(json!({"type":"resize","sessionId":sid,"width":rw,"height":rh,"scale":rs}));
            st.resizes.fetch_add(1, Ordering::Relaxed);
            let gap = if storm { rng.range(30, 150) } else { rng.range(300, 1500) };
            next_resize = Instant::now() + Duration::from_millis(gap);
        }
        if rng.below(40) == 0 {
            let _ = out_tx.send(json!({"type":"keyframe","sessionId":sid}));
        }
        tokio::time::sleep(Duration::from_millis(rng.range(10, 120))).await;
    }

    // --- ending
    match end {
        End::KillApp => {
            st.kills.fetch_add(1, Ordering::Relaxed);
            kill_testpad(slot);
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(6) {
                if closed_reason.lock().unwrap().is_some() {
                    st.kill_detected.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        End::DropWs => {
            st.ws_drops.fetch_add(1, Ordering::Relaxed);
            writer.abort(); // drops the sink: TCP closes without a WebSocket close frame
            reader.abort();
            tokio::time::sleep(Duration::from_millis(rng.range(100, 800))).await;
        }
        End::Close | End::ResizeStormThenClose => {
            let _ = out_tx.send(json!({"type":"close","sessionId":sid}));
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(5) && closed_reason.lock().unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        End::CloseWhilePending => {}
    }
    finish(pc, reader, writer).await;
    Ok(())
}

async fn worker(ctx: Arc<Ctx>, round: Arc<Round>, id: usize, deadline: Instant, seed: u64) {
    let slot = if ctx.share_exe && id == 1 { 0 } else { id };
    let mut rng = Rng(seed ^ ((id as u64 + 1) * 0x9E37_79B9_7F4A_7C15) | 1);
    while Instant::now() < deadline && !round.dead.load(Ordering::SeqCst) {
        if let Err(e) = run_cycle(&ctx, &round, id, slot, &mut rng).await {
            if !round.dead.load(Ordering::SeqCst) {
                eprintln!("[worker {id}] cycle error: {e:#}");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if rng.below(2) == 0 {
            kill_testpad(slot);
        }
        tokio::time::sleep(Duration::from_millis(rng.range(50, 600))).await;
    }
}

struct Host {
    child: Child,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

fn start_host(args: &Args, run_dir: &Path, port: u16, threads: u16, n: usize) -> Result<Host> {
    let appdata = run_dir.join(format!("appdata-{n}"));
    std::fs::create_dir_all(&appdata)?;
    let stdout_log = run_dir.join(format!("host-{n}.out.log"));
    let stderr_log = run_dir.join(format!("host-{n}.err.log"));
    let child = Command::new(&args.host_exe)
        .args(["--port", &port.to_string(), "--no-vdisplay", "--store"])
        .arg(appdata.join("host.json"))
        .arg("--verbose")
        .env("PENLINK_ENC_THREADS", threads.to_string())
        .env("APPDATA", &appdata)
        .env("RUST_BACKTRACE", "1")
        .stdout(Stdio::from(std::fs::File::create(&stdout_log)?))
        .stderr(Stdio::from(std::fs::File::create(&stderr_log)?))
        .spawn()?;
    Ok(Host { child, stdout_log, stderr_log })
}

async fn pair(host: &Host, addr: SocketAddr) -> Result<String> {
    let t = Instant::now();
    let code = loop {
        if let Ok(s) = std::fs::read_to_string(&host.stdout_log) {
            if let Some(i) = s.find("pairing code:") {
                break s[i + 13..].split_whitespace().next().unwrap_or("").to_string();
            }
        }
        if t.elapsed() > Duration::from_secs(15) {
            return Err(anyhow!("host did not print a pairing code"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let (st, body) = http(addr, "POST", "/pair", Some(&json!({"code": code, "deviceName": "stress"}).to_string())).await?;
    if st != 200 {
        return Err(anyhow!("pair failed: HTTP {st}"));
    }
    Ok(serde_json::from_str::<Value>(&body)?["token"].as_str().unwrap().to_string())
}

fn tail(path: &Path, n: usize) -> String {
    let s = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    let run_dir = args.dir.clone();
    std::fs::create_dir_all(&run_dir)?;
    for s in 0..args.workers {
        std::fs::copy(&args.testpad, run_dir.join(exe_name(s)))?;
    }
    println!("stress run dir: {}", run_dir.display());
    println!("host: {}  workers: {}  total {}s, rounds of {}s, encoder threads {:?}, seed {}", args.host_exe.display(), args.workers, args.secs, args.round_secs, args.threads, args.seed);
    let ctx = Arc::new(Ctx { stats: Stats::default(), dir: run_dir.clone(), share_exe: args.share_exe });
    let total_deadline = Instant::now() + Duration::from_secs(args.secs);
    let started = Instant::now();
    let mut n = 0usize;
    let port_base = 47900u16;
    kill_all_testpads(args.workers);

    while Instant::now() < total_deadline {
        let threads = args.threads[n % args.threads.len()];
        let port = port_base + (n as u16 % 50);
        let mut host = start_host(&args, &run_dir, port, threads, n)?;
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let token = match pair(&host, addr).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("round {n}: {e:#}; host log:\n{}", tail(&host.stderr_log, 20));
                let _ = host.child.kill();
                n += 1;
                continue;
            }
        };
        let round = Arc::new(Round { addr, token, dead: AtomicBool::new(false) });
        let round_deadline = (Instant::now() + Duration::from_secs(args.round_secs)).min(total_deadline);
        println!("--- round {n}: encoder threads {threads}, {} s ---", round_deadline.saturating_duration_since(Instant::now()).as_secs());
        let mut handles = Vec::new();
        for id in 0..args.workers {
            handles.push(tokio::spawn(worker(ctx.clone(), round.clone(), id, round_deadline, args.seed)));
        }
        // monitor the host process
        let mut crashed = None;
        while Instant::now() < round_deadline + Duration::from_secs(12) {
            match host.child.try_wait() {
                Ok(Some(status)) => {
                    crashed = Some(status);
                    break;
                }
                Ok(None) => {}
                Err(e) => eprintln!("try_wait: {e}"),
            }
            if handles.iter().all(|h| h.is_finished()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if let Some(status) = crashed {
            round.dead.store(true, Ordering::SeqCst);
            let code = status.code().map(|c| format!("{c:#x}")).unwrap_or_else(|| "?".into());
            let crash_dir = run_dir.join(format!("appdata-{n}")).join("Penlink").join("crash");
            let reports: Vec<String> = std::fs::read_dir(&crash_dir)
                .map(|rd| rd.flatten().map(|e| e.path().display().to_string()).collect())
                .unwrap_or_default();
            let summary = format!(
                "round {n} (threads {threads}) host exited with code {code} after {:?}; crash files {:?}",
                started.elapsed(),
                reports
            );
            eprintln!("!!! HOST CRASH: {summary}");
            eprintln!("--- host stderr tail ---\n{}\n--- host stdout tail ---\n{}", tail(&host.stderr_log, 40), tail(&host.stdout_log, 15));
            ctx.stats.host_crashes.lock().unwrap().push(summary);
        }
        for h in handles {
            let _ = tokio::time::timeout(Duration::from_secs(40), h).await;
        }
        if crashed.is_none() {
            let _ = host.child.kill();
            let _ = host.child.wait();
        }
        kill_all_testpads(args.workers);
        n += 1;
    }

    kill_all_testpads(args.workers);
    let s = &ctx.stats;
    let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
    println!("\n==== stress summary: {:.0} s, {} rounds ====", started.elapsed().as_secs_f64(), n);
    println!("cycles {}  opened {}  offers {}  first RTP {}  no RTP {}  conn errors {}", g(&s.cycles), g(&s.opened), g(&s.offers), g(&s.first_rtp), g(&s.no_rtp), g(&s.conn_errors));
    println!("RTP frames received {}  input msgs {}  resizes {}", g(&s.rtp_frames), g(&s.input_msgs), g(&s.resizes));
    println!("app kills {} (detected by host {})  ws drops {}  closes while pending {}", g(&s.kills), g(&s.kill_detected), g(&s.ws_drops), g(&s.early_closes));
    println!("closed reasons:");
    for (k, v) in s.reasons.lock().unwrap().iter() {
        println!("  {v:>5}  {k}");
    }
    let crashes = s.host_crashes.lock().unwrap();
    println!("host crashes: {}", crashes.len());
    for c in crashes.iter() {
        println!("  {c}");
    }
    if !crashes.is_empty() {
        std::process::exit(2);
    }
    Ok(())
}
