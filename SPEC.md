# Penlink: spec

Run one PC app full screen on a Galaxy S26 Ultra, with or without the S Pen.
Windows first, Mac later. Used mainly for coding, plus general apps.

## What the user does
1. Opens Penlink on the phone and picks their PC (reached over Tailscale).
2. Sees a grid of the PC's installed apps and taps one.
3. The app opens on the phone full screen: only that app, not the PC desktop.
4. Opens more apps; each one is a tab. Rotating the phone resizes the app.

## Parts
- `host/`: PC agent, Rust (Windows now, Mac later). One binary, `penlink-host.exe`.
- `android/`: phone app, Kotlin + Jetpack Compose. Package `dev.penlink`.
  minSdk 33, targetSdk 36. Landscape and portrait.

## Version 1 (now)
- **Capture:** the app's own window (Windows.Graphics.Capture on the HWND).
  The host resizes the window so its client area has the phone's aspect ratio,
  at the size the phone asks for. Known gap: menus and dialogs that are separate
  windows don't show. Version 2 fixes this with a virtual monitor per app.
- **Encode:** H.264 (constrained baseline) in software with `openh264`. Hardware
  encoding (Media Foundation on the Intel Arc B580) comes later.
- **Transport:** WebRTC. Host: `webrtc` crate (webrtc-rs). Phone:
  `io.getstream:stream-webrtc-android`. Video track + one data channel `input`.
  No STUN/TURN: Tailscale (or LAN) makes the host directly reachable.
- **Input injection:** `SendInput` for mouse, wheel and keys;
  `InjectSyntheticPointerInput` for pen (PT_PEN: pressure, tilt, barrel, eraser)
  and touch (PT_TOUCH). The target window is brought to the foreground first.

## Version 2
Virtual monitor per app (IddCx virtual display driver), so popups show and the
PC's own screen is untouched. Hardware encode. Mac host.

## Protocol (both sides must match this exactly)

Host listens on **TCP 47800**, plain HTTP + WebSocket (Tailscale encrypts).
It accepts connections only from loopback, Tailscale (100.64.0.0/10) and, when
started with `--allow-lan`, private LAN ranges. Android emulator reaches the
host at `10.0.2.2`, which arrives at the host as loopback.

### Pairing
- On start, if no device is paired or with `--pair`, the host prints a 6-digit
  code (valid 5 minutes).
- `POST /pair` body `{"code":"123456","deviceName":"S26 Ultra"}`
  → `200 {"token":"<random 32 bytes hex>","hostName":"DESKTOP-ABC"}` or `403`.
- Tokens are stored in `%APPDATA%\Penlink\host.json`. Every other request needs
  `Authorization: Bearer <token>` (WebSocket: query `?token=`). Wrong token → 401.

### HTTP
- `GET /info` (no auth) → `{"hostName":"...","version":"0.1.0"}`
- `GET /apps` → `[{"id":"<stable id>","name":"Visual Studio Code"}]`, from Start
  Menu shortcuts (user + all users) merged with the packaged/Store apps of
  `shell:AppsFolder` (Terminal, Calculator, new Notepad, ...). Sorted by name,
  duplicates removed (same name: the shortcut wins). Packaged apps are launched
  with `IApplicationActivationManager`.
- `GET /apps/{id}/icon` → PNG 96×96 (404 if none).
- Test ids: an `appId` of the form `exe:<path> [arguments]` launches that program
  (debug builds of the phone app can open one with `--es openAppId`).

### WebSocket `/ws?token=...` (JSON text frames)
One WebSocket per phone; each opened app is a *session* with its own
WebRTC peer connection.

Phone → host:
- `{"type":"open","appId":"...","width":1440,"height":3120,"scale":2.0}`
  width/height = video size in pixels; scale = how much bigger the phone wants
  the UI than 100% (the host sizes the window to width/scale × height/scale
  and the video stays at width×height).
- `{"type":"answer","sessionId":"...","sdp":"..."}`
- `{"type":"ice","sessionId":"...","candidate":"...","sdpMid":"0","sdpMLineIndex":0}`
- `{"type":"resize","sessionId":"...","width":3120,"height":1440,"scale":2.0}`
- `{"type":"close","sessionId":"..."}`
- `{"type":"keyframe","sessionId":"..."}` ask for a new keyframe

Host → phone:
- `{"type":"opened","sessionId":"...","appId":"...","title":"..."}` sent **immediately**
  when the host accepts an `open` (before the app is launched). It assigns the
  sessionId; `title` may be empty. The phone shows a pending tab (spinner).
- `{"type":"title","sessionId":"...","title":"..."}` optional; the window title,
  once the window has been found (sent before `offer`).
- `{"type":"offer","sessionId":"...","sdp":"..."}` (host makes the offer:
  sendonly H.264 video + data channel `input`), sent once the app window is found
  and the peer connection is ready.
- `{"type":"ice","sessionId":"...","candidate":"...","sdpMid":"0","sdpMLineIndex":0}`
- `{"type":"closed","sessionId":"...","reason":"app exited|closed|error: ..."}`
  ends a session at any point after `opened`, including a failed launch
  (`error: timed out waiting for the app window`, `error: unknown app ...`). It always
  carries a sessionId the phone already got in `opened`. The phone removes the tab
  and shows the reason as a toast. The phone may send `close` while a session is
  still pending; the host then abandons the launch (the app stays open) and answers `closed`.
- `{"type":"error","message":"..."}` for problems that belong to no session
  (bad JSON, unknown sessionId in a later message).

Order for one `open`: `opened` → (`title`) → `offer` → `ice`*, or `opened` → `closed`.

### Data channel `input` (ordered, reliable, JSON text)
All `x`,`y` are normalised 0..1 over the video frame (0,0 = top-left).
- Pen: `{"t":"pen","a":"hover|down|move|up|leave","x":0.5,"y":0.5,"p":0.6,"tx":10,"ty":-5,"b":0,"e":false}`
  `p` pressure 0..1, `tx`/`ty` tilt in degrees -90..90, `b` bit 1 = side button
  held, `e` = eraser end. Side button held while the tip goes down = right click.
- Touch (direct mode, multi-touch): `{"t":"touch","a":"down|move|up|cancel","id":0,"x":0.5,"y":0.5}`
- Mouse absolute: `{"t":"mouse","a":"move|down|up","x":0.5,"y":0.5,"btn":"left|right|middle"}`
- Mouse relative (trackpad mode): `{"t":"mrel","dx":3.5,"dy":-2}` in video pixels
- Wheel: `{"t":"wheel","dx":0,"dy":-120}` (Windows wheel units, 120 = one notch)
- Key: `{"t":"key","a":"down|up","vk":17}` Windows virtual-key code
- Text: `{"t":"text","s":"héllo"}` typed as Unicode

## Phone app screens
1. **Hosts:** add a host (address + pairing code), list saved hosts.
2. **Apps:** grid of the host's apps with icons; search box.
3. **Session:** full-screen video (immersive, no system bars), tabs for open apps
   at the top edge (auto-hide), input layer, keyboard toggle button.

## Input on the phone
| | S Pen | Finger |
|---|---|---|
| Pointer | Hover moves the cursor without clicking | Direct mode (tap = click) or trackpad mode (relative cursor, tap = click) |
| Click | Tip = left click, side button = right click | Tap; long-press or two-finger tap = right click |
| Pressure / tilt | Sent as real Windows pen input | — |
| Scroll / zoom | — | Two-finger drag = wheel; pinch = zoom the phone's view (local) |

**Keyboard: on-screen only.** Penlink's own coding keyboard: letters, numbers,
symbols, Ctrl, Alt, Shift, Win, Esc, Tab, arrows, Home/End, PgUp/PgDn, F1–F12.
Modifiers are sticky (tap once = next key, tap twice = locked). Sends `key`
messages with Windows VK codes. A button switches to the phone's normal
keyboard (sent as `text`) for long text and voice typing.

## Done means
- `cargo build --release` and `cargo test` pass in `host/`.
- `./gradlew assembleDebug` and unit tests pass in `android/`.
- End to end on the Android emulator against the real host: pair, list apps,
  open Notepad, see it, type into it, click in it.
- Then on the real S26 Ultra over Tailscale.
