//! Smoke tests against a real in-process server.
//!
//! `http_and_pairing` runs by default. `open_notepad` launches the testpad example window on this PC, so it is
//! `#[ignore]`d: run it with `cargo test --test smoke -- --ignored --nocapture`.

use futures_util::{SinkExt, StreamExt};
use penlink_host::server::{self, Config, RunningServer};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::Command;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

async fn start_server() -> RunningServer {
    let dir = std::env::temp_dir().join(format!("penlink-smoke-{}", penlink_host::store::new_code()));
    let cfg = Config {
        port: 0,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        store_path: Some(dir.join("host.json")),
        host_name: Some("SMOKEHOST".into()),
        print_code: false,
        ..Config::default()
    };
    server::start(cfg).await.expect("server starts")
}

async fn http(addr: SocketAddr, method: &str, path: &str, token: Option<&str>, body: Option<&str>) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("http header end");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut body = raw[split + 4..].to_vec();
    if head.to_lowercase().contains("transfer-encoding: chunked") {
        body = dechunk(&body);
    }
    (status, body)
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(i) = b.windows(2).position(|w| w == b"\r\n") else { break };
        let n = usize::from_str_radix(std::str::from_utf8(&b[..i]).unwrap().trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + n]);
        b = &b[i + 2 + n + 2..];
    }
    out
}

async fn pair(srv: &RunningServer) -> String {
    let code = srv.pairing_code().await.expect("a pairing code is active on a fresh store");
    assert_eq!(code.len(), 6);
    let (st, _) = http(srv.addr, "POST", "/pair", None, Some(r#"{"code":"abcdef","deviceName":"x"}"#)).await;
    assert_eq!(st, 403, "wrong code");
    let (st, body) = http(
        srv.addr,
        "POST",
        "/pair",
        None,
        Some(&json!({"code": code, "deviceName": "Smoke Phone"}).to_string()),
    )
    .await;
    assert_eq!(st, 200, "pair ok");
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["hostName"], "SMOKEHOST");
    let token = v["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);
    // the code is single-use
    let (st, _) = http(srv.addr, "POST", "/pair", None, Some(&json!({"code": code, "deviceName": "again"}).to_string())).await;
    assert_eq!(st, 403);
    token
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_and_pairing() {
    let srv = start_server().await;
    let addr = srv.addr;

    // /info needs no auth
    let (st, body) = http(addr, "GET", "/info", None, None).await;
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v, json!({"hostName":"SMOKEHOST","version":"0.1.0"}));

    // everything else needs the token
    assert_eq!(http(addr, "GET", "/apps", None, None).await.0, 401);
    assert_eq!(http(addr, "GET", "/apps", Some("wrong"), None).await.0, 401);
    assert_eq!(http(addr, "GET", "/apps/abc/icon", Some("wrong"), None).await.0, 401);

    let token = pair(&srv).await;

    let (st, body) = http(addr, "GET", "/apps", Some(&token), None).await;
    assert_eq!(st, 200);
    let apps: Vec<Value> = serde_json::from_slice(&body).unwrap();
    println!("{} apps, first few: {:?}", apps.len(), apps.iter().take(5).collect::<Vec<_>>());
    assert!(!apps.is_empty(), "Start Menu has shortcuts");
    let names: Vec<String> = apps.iter().map(|a| a["name"].as_str().unwrap().to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "sorted by name");
    let mut dedup = names.clone();
    dedup.dedup();
    assert_eq!(names.len(), dedup.len(), "no duplicate names");

    // icons: at least one app has a 96x96 PNG
    let mut ok = 0;
    for a in apps.iter().take(8) {
        let id = a["id"].as_str().unwrap();
        let (st, png) = http(addr, "GET", &format!("/apps/{id}/icon"), Some(&token), None).await;
        if st == 200 {
            assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
            let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
            let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
            assert_eq!((w, h), (96, 96));
            ok += 1;
        } else {
            assert_eq!(st, 404);
        }
    }
    println!("icons ok: {ok}/8");
    assert!(ok > 0, "no icon could be extracted");
    assert_eq!(http(addr, "GET", "/apps/doesnotexist/icon", Some(&token), None).await.0, 404);

    // websocket auth
    let bad = tokio_tungstenite::connect_async(format!("ws://{addr}/ws?token=nope")).await;
    assert!(bad.is_err(), "ws with a bad token must be rejected");
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws?token={token}")).await.unwrap();
    ws.send(Message::Text(r#"{"type":"keyframe","sessionId":"nope"}"#.into())).await.unwrap();
    ws.send(Message::Text(r#"{"type":"open","appId":"unknown-app","width":720,"height":1280,"scale":1.0}"#.into())).await.unwrap();
    // `opened` always comes first (so the phone knows the session), then `closed` with that id.
    let opened = next_json(&mut ws, 5).await;
    assert_eq!(opened["type"], "opened", "{opened}");
    let sid = opened["sessionId"].as_str().unwrap().to_string();
    let closed = next_json(&mut ws, 5).await;
    assert_eq!(closed["type"], "closed", "{closed}");
    assert_eq!(closed["sessionId"], sid.as_str(), "closed must carry the session id the phone was given");
    assert!(closed["reason"].as_str().unwrap().contains("unknown app"), "{closed}");

    // A launch that fails (program does not exist) behaves the same way.
    ws.send(Message::Text(
        json!({"type":"open","appId":"exe:C:\\penlink-no-such-dir\\nope.exe","width":720,"height":1280,"scale":1.0}).to_string().into(),
    ))
    .await
    .unwrap();
    let opened = next_json(&mut ws, 5).await;
    assert_eq!(opened["type"], "opened", "{opened}");
    let sid = opened["sessionId"].as_str().unwrap().to_string();
    let closed = next_json(&mut ws, 20).await;
    assert_eq!(closed["type"], "closed", "{closed}");
    assert_eq!(closed["sessionId"], sid.as_str());
    assert!(closed["reason"].as_str().unwrap().starts_with("error:"), "{closed}");
    srv.shutdown().await;
}

fn notepad_pids() -> Vec<u32> {
    let out = Command::new("tasklist").args(["/FI", "IMAGENAME eq testpad.exe", "/FO", "CSV", "/NH"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split("\",\"").nth(1).and_then(|p| p.trim_matches('"').parse().ok()))
        .collect()
}

async fn next_json(ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin), secs: u64) -> Value {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(secs), ws.next()).await.expect("timeout").expect("ws open").unwrap();
        if let Message::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "launches the testpad example window on this PC"]
async fn open_notepad() {
    let srv = start_server().await;
    let token = pair(&srv).await;
    // Never a normal user app: the host would reuse (and type into) an already-running one.
    // Needs `cargo build --example testpad` first.
    let exe = concat!(env!("CARGO_MANIFEST_DIR"), "/target/debug/examples/testpad.exe").replace('/', "\\");
    assert!(std::path::Path::new(&exe).exists(), "run `cargo build --example testpad` first");
    let app_id = format!("exe:{exe} {}", std::env::temp_dir().join("penlink-smoke-testpad.log").display());
    println!("opening app id {app_id}");

    let before = notepad_pids();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws?token={token}", srv.addr)).await.unwrap();
    ws.send(Message::Text(json!({"type":"open","appId":app_id,"width":1440,"height":3120,"scale":2.0}).to_string().into()))
        .await
        .unwrap();

    let opened = next_json(&mut ws, 25).await;
    assert_eq!(opened["type"], "opened", "{opened}");
    let sid = opened["sessionId"].as_str().unwrap().to_string();
    println!("opened: {opened}");
    let offer = next_json(&mut ws, 15).await;
    assert_eq!(offer["type"], "offer");
    assert_eq!(offer["sessionId"], sid);
    let sdp = offer["sdp"].as_str().unwrap();
    println!("---- offer SDP ----\n{sdp}\n-------------------");
    assert!(sdp.contains("m=video"), "video m-line");
    assert!(sdp.to_uppercase().contains("H264/90000"), "H264 codec");
    assert!(sdp.contains("profile-level-id=42e01f"));
    assert!(sdp.contains("packetization-mode=1"));
    assert!(sdp.contains("a=sendonly"), "sendonly video");
    assert!(sdp.contains("m=application") && sdp.contains("webrtc-datachannel"), "data channel m-line");

    // at least one trickled local candidate (loopback or interface)
    let mut candidates = Vec::new();
    for _ in 0..30 {
        let Ok(Some(Ok(Message::Text(t)))) = tokio::time::timeout(Duration::from_secs(3), ws.next()).await else { break };
        let v: Value = serde_json::from_str(&t).unwrap();
        if v["type"] == "ice" {
            candidates.push(v["candidate"].as_str().unwrap().to_string());
        }
    }
    println!("{} local candidates:\n{}", candidates.len(), candidates.join("\n"));
    assert!(!candidates.is_empty());
    assert!(candidates.iter().any(|c| c.contains(" 127.0.0.1 ")), "loopback candidate offered");

    ws.send(Message::Text(json!({"type":"close","sessionId":sid}).to_string().into())).await.unwrap();
    let closed = loop {
        let v = next_json(&mut ws, 10).await;
        if v["type"] == "closed" {
            break v;
        }
    };
    assert_eq!(closed["reason"], "closed");
    srv.shutdown().await;

    // Only terminate testpad processes this test started.
    tokio::time::sleep(Duration::from_millis(500)).await;
    for pid in notepad_pids().into_iter().filter(|p| !before.contains(p)) {
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output();
    }
}
