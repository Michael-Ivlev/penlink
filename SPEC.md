# Penlink: spec

Run one PC app full screen on a Galaxy S26 Ultra, with or without the S Pen.
Windows first, Mac later. Used mainly for coding, plus general apps.

## What the user does
1. Opens Penlink on the phone and picks their PC (reached over Tailscale only).
2. Sees a grid of the PC's installed apps and taps one.
3. The app opens on the phone full screen: only that app, not the PC desktop.
4. Opens more apps; each one is a tab. Rotating the phone resizes the app.
5. Meanwhile the PC's own screen stays free to use.

## How it works
- **One virtual monitor per app.** The PC agent creates a virtual monitor shaped
  like the phone (resolution, orientation, text scaling), launches the app on it
  maximised, and streams only that monitor. The app's menus, dialogs and file
  pickers open on the same monitor, so they show up on the phone too.
- **Network:** Tailscale only. The phone connects to the PC by its Tailscale name.
  A one-time pairing token stops other devices on the tailnet from connecting.
- **Stream:** WebRTC. The PC encodes in hardware; the phone decodes in hardware.
  Tuned for sharp text first, then latency: full phone resolution, keep
  resolution when bandwidth drops, lower frame rate when nothing moves.
  Target: under 50 ms from PC to phone screen on a good connection.
- **Input** goes back over a WebRTC data channel.

## Input
| | S Pen | Finger |
|---|---|---|
| Pointer | Hover moves the cursor without clicking | Direct mode (tap = click) or trackpad mode (relative cursor) |
| Click | Tip = left click, side button = right click | Tap; long-press or two-finger tap = right click |
| Pressure / tilt | Sent as real Windows pen input | — |
| Scroll / zoom | — | Two-finger scroll goes to the app; pinch zooms the phone's view |

**Keyboard: on-screen only, no Bluetooth keyboard.** Penlink has its own keyboard
built for coding: letters plus Ctrl, Alt, Shift, Win, Esc, Tab, arrows, F-keys and
symbol keys, with sticky modifiers so Ctrl+Shift+P works with one thumb. It sends
exact key codes. A button switches to the phone's normal keyboard for long text
and voice typing.

## Parts
- **Phone app:** React Native (Expo dev build), TypeScript.
  - Video and data channel: `react-native-webrtc`.
  - Pen and touch: `react-native-gesture-handler` (hover, pan, pinch; `stylusData`
    gives pressure and tilt).
  - Keyboard: our own React Native component.
  - Native Kotlin only where a library falls short (see Spike).
- **PC agent:** Rust. Windows: virtual display driver (IddCx), Windows.Graphics.Capture,
  hardware encoder, WebRTC, `SendInput` for keys and mouse, `InjectSyntheticPointerInput`
  for pen and touch. Mac later: virtual display or ScreenCaptureKit, CGEvent input.

## Spike first (1–2 days)
Before building the rest, prove on the real phone:
1. Video from the PC appears through `react-native-webrtc` and text stays readable.
2. Gesture Handler gets S Pen hover, pressure, tilt and the **side button** on the S26 Ultra.
3. Time from pen movement to cursor movement on the PC through JavaScript is acceptable.

Anything that fails becomes a small native Kotlin module for that one piece.

## First version (Windows)
1. PC agent lists installed apps and launches one on a phone-shaped virtual monitor.
2. Stream to the phone, sharp enough to read code.
3. Finger (direct + trackpad), S Pen hover and clicks, on-screen coding keyboard.
4. Several apps as tabs; rotation resizes the app.

## Later
Mac agent. Clipboard sync. File drag between phone and PC.
