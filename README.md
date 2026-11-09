# Penlink

Use your PC's apps from an Android phone as if they were phone apps.

Remote desktop on a phone usually means your whole monitor shrunk onto a small screen, and you pinch and zoom around
it. Penlink works per app instead. You pick an app from a grid on the phone, the PC creates a virtual monitor at the
phone's exact screen size, and the app opens full screen on it. The app lays itself out for the phone, and you still
get the real desktop program with all its menus and shortcuts.

<p align="center">
  <img src="docs/screenshots/1-pcs.png" width="24%" alt="Your PCs">
  <img src="docs/screenshots/2-apps.png" width="24%" alt="Apps on the PC">
  <img src="docs/screenshots/3-session.png" width="24%" alt="An app running full screen with the pen">
  <img src="docs/screenshots/4-notifications.png" width="24%" alt="PC notifications on the phone">
</p>

<sub>These are illustrations of the app's screens. Real captures from the end-to-end tests are in
[`docs/e2e/`](docs/e2e).</sub>

## What it does

- Each app gets its own virtual monitor sized to the phone. Rotate the phone and the app re-lays itself out; open the
  keyboard and it fits in the space above it.
- Your real monitors aren't changed. When you close the app on the phone, its virtual monitor is removed and the window
  goes back to your main screen.
- The S Pen reaches Windows as a pen, not a mouse: pressure, tilt, the barrel button and the eraser all come through,
  so Windows Ink and drawing apps work like they do with a pen tablet. Palm rejection ignores your hand while the pen
  is near the screen.
- Touch is sent as Windows touch, or you can switch to trackpad mode. The keyboard has an extra row for Esc, Tab,
  Ctrl, Alt, Win and the arrows, so shortcuts work.
- The phone shows a searchable grid of the PC's apps (Start Menu and Store apps). You can have several open and switch
  between them.
- Menus, dialogs and file pickers show up on the phone too, because they open on the same virtual monitor.
- A UI size slider makes the interface bigger for a phone screen, saved per PC.
- PC notifications are forwarded to the phone.
- Latency is low: video is H.264 over WebRTC and input goes back on a data channel. I measured about 27 ms from touch
  to pixel on a local connection. There's an optional stats overlay that breaks the time down.
- The host only accepts connections from the PC itself and from Tailscale (LAN is opt-in). The phone pairs with a
  one-time 6-digit code, and there's no cloud service in between.

## How it works

```
Phone (Kotlin, Jetpack Compose)                 PC (Rust, penlink-host.exe)
  pick PC -> pick app  ── HTTP/WebSocket ──▶  launch app, add virtual monitor
  video view           ◀── WebRTC H.264 ───  capture monitor, encode (openh264)
  pen/touch/keys       ── data channel ───▶  SendInput / InjectSyntheticPointerInput
```

The virtual screens come from the SudoVDA display driver, which the host installs on first run (it asks once). Without
it, Penlink falls back to capturing the app's window. The full protocol and behaviour are described in [`SPEC.md`](SPEC.md).

## Getting started

You need a Windows 10/11 PC, an Android 13+ phone, and both on the same Tailscale network (or LAN).

1. Download `penlink-host.exe` and `Penlink.apk` from the [latest release](https://github.com/Michael-Ivlev/penlink/releases/latest).
2. **PC:** run `penlink-host.exe`. It prints a 6-digit pairing code. On first start it asks once (UAC) to install the
   virtual display driver. Allow UDP ports 47801-48800 in Windows Firewall for your Tailscale/LAN profile.
3. **Phone:** install `Penlink.apk` (allow installs from your browser or file manager when Android asks).
4. In the app tap **Add PC**, enter the PC's Tailscale address and the code, then tap an app.

### Building from source

```bash
cd host
cargo build --release          # target/release/penlink-host.exe
```

```bash
cd android
./gradlew assembleDebug        # app/build/outputs/apk/debug/app-debug.apk
```

More detail in [`host/README.md`](host/README.md) and [`android/README.md`](android/README.md).

## Repository layout

| Path       | What                                                        |
|------------|-------------------------------------------------------------|
| `host/`    | Windows agent in Rust: capture, encode, WebRTC, input, driver |
| `android/` | Phone app in Kotlin + Jetpack Compose (package `dev.penlink`) |
| `docs/`    | Screenshots and end-to-end test captures                    |
| `SPEC.md`  | Protocol and behaviour spec                                 |

## Status

Early, but it works day to day. Windows host only, and video is encoded in software for now; hardware encoding and a
macOS host are planned. Mostly tested on a Galaxy S26 Ultra, so reports from other phones are very welcome.

## License

MIT, see [`LICENSE`](LICENSE). The host embeds the SudoVDA driver (MIT) and nefcon (BSD-3-Clause); see
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).
