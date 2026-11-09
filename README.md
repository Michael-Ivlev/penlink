# Penlink

**Remote control for your PC, where every desktop app looks and works like a phone app.**

Penlink gives you your PC from your Android phone, but not as a tiny desktop you pinch and zoom around. You pick an
app from an app grid and it opens full screen, laid out for the phone, and you use it like any other phone app. It is
still the real desktop program, with every menu, panel, shortcut and plugin it has on your PC, so you have full
control and nothing is cut down.

<p align="center">
  <img src="docs/screenshots/1-pcs.png" width="24%" alt="Your PCs">
  <img src="docs/screenshots/2-apps.png" width="24%" alt="Apps on the PC">
  <img src="docs/screenshots/3-session.png" width="24%" alt="An app running full screen with the pen">
  <img src="docs/screenshots/4-notifications.png" width="24%" alt="PC notifications on the phone">
</p>

<sub>The images above are illustrations of the app's screens. Real captures from the end-to-end tests are in
[`docs/e2e/`](docs/e2e).</sub>

## Why Penlink

### 📱 Desktop apps that feel native on mobile

- **Made to the phone's shape.** Each app gets a screen on the PC created at the exact size and shape of your phone's
  screen. The app lays itself out for the phone, so nothing is shrunk, letterboxed or cut off.
- **Follows the phone.** Rotate it and the app re-lays itself out in portrait or landscape. Open the keyboard and the
  app fits the space above it.
- **Readable, touchable UI.** A UI size slider scales the interface up for a phone screen, saved per PC.
- **Apps, not a desktop.** A searchable grid of the PC's apps. Open several and switch between them like phone apps;
  minimise and close them from the phone.
- **Popups behave.** Menus, dialogs and file pickers show up on the phone too.
- **Notifications on the phone.** The PC's notifications arrive as phone notifications, per app.

### 🖊️ S Pen as a real Windows pen

The S Pen is not turned into a mouse. It reaches Windows as a genuine pen, with **pressure, tilt, the barrel button
and the eraser**, so drawing apps, handwriting, Windows Ink and pen gestures work as they would with a pen tablet.
Palm rejection ignores your hand while the pen is near the screen.

### ⚡ Blazing fast

Video goes over WebRTC with low-latency H.264, and input goes back over a WebRTC data channel. Touch-to-pixel latency
was measured at about **27 ms** over a local connection, so the app feels like it is running on the phone. An
optional stats overlay shows where every millisecond goes.

### 🖥️ Doesn't take over your PC

Every app you open from the phone gets **its own virtual screen** on the PC. Your real monitors keep their layout and
resolution, and the apps on the phone never cover what is on them. Close the app on the phone and its virtual screen disappears and
the window goes back to your main monitor.

### 🎛️ Full control

- **Touch** goes to Windows as real touch, or switch to trackpad mode.
- **Full keyboard:** the phone keyboard plus an extra-keys row (Esc, Tab, Ctrl, Alt, Win, arrows), so every shortcut
  works.
- **Any app:** anything in the PC's Start Menu or Store apps, from an editor and a terminal to design and office apps.

### 🔒 Private by default

The PC only answers on its own machine and on Tailscale (LAN is opt-in), and the phone pairs with a one-time 6-digit
code. No cloud service in between.

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

1. **PC:** build and run the host (see [`host/README.md`](host/README.md)):
   ```bash
   cd host
   cargo build --release
   target/release/penlink-host.exe
   ```
   It prints a 6-digit pairing code. On first start it asks once (UAC) to install the virtual display driver.
2. **Phone:** build and install the app (see [`android/README.md`](android/README.md)):
   ```bash
   cd android
   ./gradlew assembleDebug
   adb install -r app/build/outputs/apk/debug/app-debug.apk
   ```
3. In the app tap **Add PC**, enter the PC's Tailscale address and the code, then tap an app.

## Repository layout

| Path       | What                                                        |
|------------|-------------------------------------------------------------|
| `host/`    | Windows agent in Rust: capture, encode, WebRTC, input, driver |
| `android/` | Phone app in Kotlin + Jetpack Compose (package `dev.penlink`) |
| `docs/`    | Screenshots and end-to-end test captures                    |
| `SPEC.md`  | Protocol and behaviour spec                                 |

## Status

Windows host and Android app work today. Hardware video encoding and a macOS host are planned.

## License

MIT, see [`LICENSE`](LICENSE). The host embeds the SudoVDA driver (MIT) and nefcon (BSD-3-Clause); see
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).
