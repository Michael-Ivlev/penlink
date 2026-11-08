# penlink-host

Windows PC agent for Penlink: streams one app window to the phone over WebRTC
(H.264 via openh264) and injects mouse, keyboard, pen and touch input. Protocol: see `../SPEC.md`.

## Build / test
    cargo build --release          # target\release\penlink-host.exe
    cargo test                     # unit + HTTP/pairing smoke test
    cargo test --test smoke -- --ignored --nocapture   # launches Notepad, checks offer SDP
    cargo run --release --example client -- --png out.png   # loopback client: receives + decodes the stream

## Run
    penlink-host.exe [--pair] [--allow-lan] [--port 47800] [--verbose]

- Listens on TCP 47800 (HTTP + WebSocket). Accepts loopback, Tailscale (100.64.0.0/10) and, with
  `--allow-lan`, private LAN ranges. Everything else gets 403.
- Pairing: if no phone is paired (or with `--pair`) a 6-digit code is printed (valid 5 min, single use,
  5 wrong guesses burn it). The phone POSTs it to `/pair` and receives a token, stored in
  `%APPDATA%\Penlink\host.json`.
- WebRTC media uses UDP ports 47801-48800 (allow them in Windows Firewall for the Tailscale/LAN profile).
- `appId` may also be `exe:<path or name>` (e.g. `exe:notepad.exe`) for testing.
- Opening an app resizes its window (client area = width/scale x height/scale, on the primary monitor).
  Closing a session never kills the app.
