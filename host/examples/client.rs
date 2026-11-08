//! Loopback test client: pairs, opens an app, answers the host's WebRTC offer, receives the
//! H.264 RTP stream, decodes it with openh264 and reports frames / bytes / fps / bitrate.
//!
//! Without --host an in-process server is started on a random port.
//!
//!   cargo run --release --example client -- [--host 127.0.0.1:47800 --code 123456]
//!       [--app exe:notepad.exe] [--size 1440x3120] [--scale 2] [--secs 5] [--png out.png]
//!       [--keep]   (do not terminate the Notepad processes this run started)

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::process::Command;
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
use webrtc::rtp::codecs::h264::H264Packet;
use webrtc::rtp::packetizer::Depacketizer;

#[derive(Default)]
struct Counters {
    packets: AtomicU64,
    bytes: AtomicU64,
    frames: AtomicU64,
    decoded: AtomicU64,
    idr: AtomicU64,
    first_packet: AtomicBool,
}

struct Args {
    host: Option<String>,
    code: Option<String>,
    app: String,
    w: u32,
    h: u32,
    scale: f64,
    secs: u64,
    png: Option<String>,
    keep: bool,
}

fn parse_args() -> Args {
    let mut a = Args { host: None, code: None, app: "exe:notepad.exe".into(), w: 1440, h: 3120, scale: 2.0, secs: 5, png: None, keep: false };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--host" => a.host = it.next(),
            "--code" => a.code = it.next(),
            "--app" => a.app = it.next().unwrap(),
            "--size" => {
                let v = it.next().unwrap();
                let (w, h) = v.split_once('x').unwrap();
                a.w = w.parse().unwrap();
                a.h = h.parse().unwrap();
            }
            "--scale" => a.scale = it.next().unwrap().parse().unwrap(),
            "--secs" => a.secs = it.next().unwrap().parse().unwrap(),
            "--png" => a.png = it.next(),
            "--keep" => a.keep = true,
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

fn notepad_pids() -> Vec<u32> {
    let out = Command::new("tasklist").args(["/FI", "IMAGENAME eq Notepad.exe", "/FO", "CSV", "/NH"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split("\",\"").nth(1).and_then(|p| p.trim_matches('"').parse().ok()))
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    let mut inproc = None;
    let (addr, code): (SocketAddr, String) = match &args.host {
        Some(h) => (h.parse()?, args.code.clone().expect("--code required with --host")),
        None => {
            let dir = std::env::temp_dir().join(format!("penlink-client-{}", penlink_host::store::new_code()));
            let srv = penlink_host::server::start(penlink_host::server::Config {
                port: 0,
                bind: std::net::Ipv4Addr::LOCALHOST.into(),
                store_path: Some(dir.join("host.json")),
                print_code: false,
                ..Default::default()
            })
            .await?;
            let c = srv.pairing_code().await.unwrap();
            let a = srv.addr;
            inproc = Some(srv);
            (a, c)
        }
    };
    println!("host {addr}");

    // pair
    let (st, body) = http(addr, "POST", "/pair", Some(&json!({"code": code, "deviceName": "loopback client"}).to_string())).await?;
    if st != 200 {
        return Err(anyhow!("pairing failed with HTTP {st}"));
    }
    let token = serde_json::from_str::<Value>(&body)?["token"].as_str().unwrap().to_string();
    println!("paired");

    // peer connection (answerer)
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media)?;
    let mut se = SettingEngine::default();
    se.set_include_loopback_candidate(true);
    se.set_network_types(vec![NetworkType::Udp4]);
    let api = APIBuilder::new().with_media_engine(media).with_interceptor_registry(registry).with_setting_engine(se).build();
    let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    let (ws_out_tx, mut ws_out_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let session_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    {
        let tx = ws_out_tx.clone();
        let sid = session_id.clone();
        pc.on_ice_candidate(Box::new(move |c| {
            let tx = tx.clone();
            let sid = sid.clone();
            Box::pin(async move {
                if let (Some(c), Some(sid)) = (c, sid.lock().unwrap().clone()) {
                    let j = c.to_json().unwrap();
                    let _ = tx.send(json!({"type":"ice","sessionId":sid,"candidate":j.candidate,"sdpMid":j.sdp_mid,"sdpMLineIndex":j.sdp_mline_index}));
                }
            })
        }));
    }

    let counters = Arc::new(Counters::default());
    let last_frame: Arc<Mutex<Option<(usize, usize, Vec<u8>)>>> = Arc::new(Mutex::new(None));
    let stream_dims: Arc<Mutex<Option<(usize, usize)>>> = Arc::new(Mutex::new(None));
    {
        let counters = counters.clone();
        let last_frame = last_frame.clone();
        let dims = stream_dims.clone();
        pc.on_track(Box::new(move |track, _rx, _tr| {
            let counters = counters.clone();
            let last_frame = last_frame.clone();
            let dims = dims.clone();
            Box::pin(async move {
                println!("on_track: {} {}", track.kind(), track.codec().capability.mime_type);
                tokio::spawn(async move {
                    let mut dep = H264Packet::default();
                    let mut au: Vec<u8> = Vec::new();
                    let mut dec = openh264::decoder::Decoder::new().expect("decoder");
                    while let Ok((pkt, _)) = track.read_rtp().await {
                        counters.first_packet.store(true, Ordering::SeqCst);
                        counters.packets.fetch_add(1, Ordering::Relaxed);
                        counters.bytes.fetch_add(pkt.payload.len() as u64, Ordering::Relaxed);
                        if let Ok(b) = dep.depacketize(&pkt.payload) {
                            au.extend_from_slice(&b);
                        }
                        if pkt.header.marker {
                            counters.frames.fetch_add(1, Ordering::Relaxed);
                            if penlink_host::encode::nal_types(&au).contains(&5) {
                                counters.idr.fetch_add(1, Ordering::Relaxed);
                            }
                            use openh264::formats::YUVSource;
                            if let Ok(Some(y)) = dec.decode(&au) {
                                counters.decoded.fetch_add(1, Ordering::Relaxed);
                                let (w, h) = y.dimensions();
                                let mut rgb = vec![0u8; w * h * 3];
                                y.write_rgb8(&mut rgb);
                                *dims.lock().unwrap() = Some((w, h));
                                *last_frame.lock().unwrap() = Some((w, h, rgb));
                            }
                            au.clear();
                        }
                    }
                });
            })
        }));
    }
    let dc_open = Arc::new(AtomicBool::new(false));
    let dc_slot: Arc<Mutex<Option<Arc<webrtc::data_channel::RTCDataChannel>>>> = Arc::new(Mutex::new(None));
    {
        let dc_open = dc_open.clone();
        let slot = dc_slot.clone();
        pc.on_data_channel(Box::new(move |dc| {
            let dc_open = dc_open.clone();
            let slot = slot.clone();
            Box::pin(async move {
                println!("data channel '{}' announced", dc.label());
                let d2 = dc.clone();
                dc.on_open(Box::new(move || {
                    dc_open.store(true, Ordering::SeqCst);
                    Box::pin(async {})
                }));
                *slot.lock().unwrap() = Some(d2);
            })
        }));
    }

    // websocket
    let before = notepad_pids();
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws?token={token}")).await?;
    let (mut ws_tx, mut ws_rx) = ws.split();
    tokio::spawn(async move {
        while let Some(v) = ws_out_rx.recv().await {
            if ws_tx.send(Message::Text(v.to_string().into())).await.is_err() {
                break;
            }
        }
    });
    ws_out_tx.send(json!({"type":"open","appId":args.app,"width":args.w,"height":args.h,"scale":args.scale}))?;
    let opened_at = Instant::now();

    // signalling reader
    let pc2 = pc.clone();
    let sid2 = session_id.clone();
    let tx2 = ws_out_tx.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let closed2 = closed.clone();
    let sig = tokio::spawn(async move {
        let mut pending: Vec<RTCIceCandidateInit> = Vec::new();
        let mut remote_set = false;
        while let Some(Ok(m)) = ws_rx.next().await {
            let Message::Text(t) = m else { continue };
            let v: Value = serde_json::from_str(&t).unwrap();
            match v["type"].as_str().unwrap() {
                "opened" => {
                    println!("opened: title={:?} after {:?}", v["title"], opened_at.elapsed());
                    *sid2.lock().unwrap() = Some(v["sessionId"].as_str().unwrap().to_string());
                }
                "offer" => {
                    let offer = RTCSessionDescription::offer(v["sdp"].as_str().unwrap().to_string()).unwrap();
                    pc2.set_remote_description(offer).await.unwrap();
                    let ans = pc2.create_answer(None).await.unwrap();
                    let sdp = ans.sdp.clone();
                    pc2.set_local_description(ans).await.unwrap();
                    let sid = sid2.lock().unwrap().clone().unwrap();
                    let _ = tx2.send(json!({"type":"answer","sessionId":sid,"sdp":sdp}));
                    remote_set = true;
                    for c in pending.drain(..) {
                        let _ = pc2.add_ice_candidate(c).await;
                    }
                }
                "ice" => {
                    let c = RTCIceCandidateInit {
                        candidate: v["candidate"].as_str().unwrap().to_string(),
                        sdp_mid: v["sdpMid"].as_str().map(|s| s.to_string()),
                        sdp_mline_index: v["sdpMLineIndex"].as_u64().map(|n| n as u16),
                        username_fragment: None,
                    };
                    if remote_set {
                        let _ = pc2.add_ice_candidate(c).await;
                    } else {
                        pending.push(c);
                    }
                }
                "closed" | "error" => {
                    println!("host says: {v}");
                    closed2.store(true, Ordering::SeqCst);
                }
                other => println!("host message {other}"),
            }
        }
    });

    // wait for media
    let deadline = Instant::now() + Duration::from_secs(25);
    while !counters.first_packet.load(Ordering::SeqCst) {
        if Instant::now() > deadline || closed.load(Ordering::SeqCst) {
            return Err(anyhow!("no RTP received (connection state {})", pc.connection_state()));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("first RTP after {:?} (connection {})", opened_at.elapsed(), pc.connection_state());

    let snap = |c: &Counters| (c.packets.load(Ordering::Relaxed), c.bytes.load(Ordering::Relaxed), c.frames.load(Ordering::Relaxed), c.decoded.load(Ordering::Relaxed));
    let send_input = |msg: Value| {
        let dc = dc_slot.lock().unwrap().clone();
        async move {
            if let Some(dc) = dc {
                let _ = dc.send_text(msg.to_string()).await;
            }
        }
    };
    for _ in 0..40 {
        if dc_open.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("input data channel open: {}", dc_open.load(Ordering::SeqCst));

    // Phase 1: idle (static window)
    let idle_secs = (args.secs / 2).max(2);
    let s0 = snap(&counters);
    let t0 = Instant::now();
    // click into the document, then type one line
    send_input(json!({"t":"mouse","a":"move","x":0.5,"y":0.4})).await;
    send_input(json!({"t":"mouse","a":"down","x":0.5,"y":0.4,"btn":"left"})).await;
    send_input(json!({"t":"mouse","a":"up","x":0.5,"y":0.4,"btn":"left"})).await;
    send_input(json!({"t":"text","s":"Penlink loopback test: typed over the input data channel.\n"})).await;
    tokio::time::sleep(Duration::from_secs(idle_secs)).await;
    let s1 = snap(&counters);
    let d1 = t0.elapsed().as_secs_f64();

    // Phase 2: typing stress (continuous screen changes)
    let t1 = Instant::now();
    let typing = args.secs;
    let mut n = 0u32;
    while t1.elapsed() < Duration::from_secs(typing) {
        send_input(json!({"t":"text","s": format!("{:04} the quick brown fox jumps over the lazy dog\n", n)})).await;
        n += 1;
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let s2 = snap(&counters);
    let d2 = t1.elapsed().as_secs_f64();

    let report = |name: &str, a: (u64, u64, u64, u64), b: (u64, u64, u64, u64), secs: f64| {
        println!(
            "{name:<22} {:>6.1}s  packets {:>6}  frames {:>5} ({:>5.1} fps)  decoded {:>5}  payload {:>8} B  ({:>6.2} Mbps)",
            secs,
            b.0 - a.0,
            b.2 - a.2,
            (b.2 - a.2) as f64 / secs,
            b.3 - a.3,
            b.1 - a.1,
            (b.1 - a.1) as f64 * 8.0 / secs / 1e6
        );
    };
    println!("---- results ----");
    report("idle (after 1 click)", s0, s1, d1);
    report("typing stress", s1, s2, d2);
    println!("keyframes received (IDR access units): {}", counters.idr.load(Ordering::Relaxed));
    println!("decoded video size: {:?}", stream_dims.lock().unwrap());
    println!("peer connection state: {}", pc.connection_state());
    if let Some(path) = &args.png {
        if let Some((w, h, rgb)) = last_frame.lock().unwrap().clone() {
            let mut out = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
                enc.set_color(png::ColorType::Rgb);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header()?.write_image_data(&rgb)?;
            }
            std::fs::write(path, out)?;
            println!("last decoded frame written to {path} ({w}x{h})");
        }
    }

    // test resize path: portrait -> landscape
    let sid = session_id.lock().unwrap().clone().unwrap();
    ws_out_tx.send(json!({"type":"resize","sessionId":sid,"width":args.h,"height":args.w,"scale":args.scale}))?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("after resize -> decoded video size: {:?}", stream_dims.lock().unwrap());
    if let Some(path) = &args.png {
        if let Some((w, h, rgb)) = last_frame.lock().unwrap().clone() {
            let mut out = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
                enc.set_color(png::ColorType::Rgb);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header()?.write_image_data(&rgb)?;
            }
            std::fs::write(path.replace(".png", "-landscape.png"), out)?;
        }
    }

    ws_out_tx.send(json!({"type":"close","sessionId":sid}))?;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let _ = pc.close().await;
    sig.abort();
    if let Some(s) = inproc {
        s.shutdown().await;
    }
    if !args.keep {
        for pid in notepad_pids().into_iter().filter(|p| !before.contains(p)) {
            let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output();
            println!("terminated Notepad pid {pid} started by this run");
        }
    }
    Ok(())
}
