# penlink-host

Windows PC agent for Penlink: streams one app window to the phone over WebRTC
(H.264 via openh264) and injects mouse, keyboard, pen and touch input. Protocol: see `../SPEC.md`.

## Build / test
    cargo build --release          # target\release\penlink-host.exe
    cargo test                     # unit + HTTP/pairing smoke test
    cargo test --test smoke -- --ignored --nocapture   # launches the testpad example, checks offer SDP (build it first: cargo build --example testpad)
    cargo run --release --example client -- --png out.png   # loopback client: receives + decodes the stream

## Run
    penlink-host.exe [--pair] [--allow-lan] [--port 47800] [--verbose]

- Listens on TCP 47800 (HTTP + WebSocket). Accepts loopback, Tailscale (100.64.0.0/10) and, with
  `--allow-lan`, private LAN ranges. Everything else gets 403.
- Pairing: if no phone is paired (or with `--pair`) a 6-digit code is printed (valid 5 min, single use,
  5 wrong guesses burn it). The phone POSTs it to `/pair` and receives a token, stored in
  `%APPDATA%\Penlink\host.json`.
- WebRTC media uses UDP ports 47801-48800 (allow them in Windows Firewall for the Tailscale/LAN profile).
- `appId` may also be `exe:<path or name> [arguments]` (e.g. `exe:C:\tools\testpad.exe C:\log.txt`) for testing.
  `/apps` lists Start Menu shortcuts plus packaged (Store) apps from `shell:AppsFolder`; a shortcut wins over a packaged
  entry with the same name, packaged apps are launched with `IApplicationActivationManager`.
- Opening an app resizes its window (client area = width/scale x height/scale, on the primary monitor).
  Closing a session never kills the app.

## Notes from the end-to-end run

- Printable keys from the phone keyboard (`key` messages, US-layout VK codes) are typed as Unicode when no
  Ctrl/Alt/Win is held, so a PC with another active input language (for example Hebrew) still types what the
  phone key shows. Shortcuts (Ctrl+A ...) stay real key events.
- Input is dropped, not injected, while the streamed window is not the foreground window (SendInput would
  otherwise go to whatever app the user switched to). Key-ups and mouse-ups are always sent.
- Encoder: openh264, 4 threads, 1200-byte slices, camera real-time usage, low complexity (see `examples/encbench.rs`).
- `cargo run --release --example testpad -- <log file>`: test window that logs every key/mouse/pen event it gets.

## Crash reports and stress test

- An unhandled exception (for example an access violation) writes `%APPDATA%\Penlink\crash\crash-<time>-<pid>.txt`
  (code, `module+offset`, unwound stack) and a `.dmp` minidump. Map offsets with the PDB (release builds have line tables).
- `cargo run --release --example stress -- --secs 300 --workers 4 --round-secs 75 --threads 4,2,1,3 [--share-exe]`
  starts `penlink-host.exe` as a child (own token store) and drives it through HTTP/WebSocket/WebRTC with several
  concurrent testpad sessions: random sizes/scales, resize storms, input of every kind, closing, killing the app,
  dropping the WebSocket, closing while launching, encoder thread counts per round (`PENLINK_ENC_THREADS`).
  It only opens and kills copies named `plstress-tp<N>.exe` that it makes itself. Build the host and examples first.
- `/open` sizes above 3840x2160 (or 2160x3840) are scaled down to that box (openh264 limit).
