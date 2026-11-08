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
Hardware encode and the Mac host come later. The virtual monitor per app is built:

- **Driver:** SudoVDA (IddCx), embedded in `penlink-host.exe` and installed by it (see "Driver install" below). The host opens
  device interface `{e5bcc234-1e0c-418a-a0d4-ef8b7501414d}`, checks the protocol version, pings the driver
  watchdog three times per timeout (host dies: its monitors vanish after about 3 s) and pins the render
  adapter to the first hardware GPU (`PENLINK_RENDER_ADAPTER` overrides by name).
- **When:** the driver is usable and the `open` carries `quality` or `uiScale` (and the host was not started with
  `--no-vdisplay`). Otherwise, or if any step fails, version 1 window capture is used and `window.mode` says `window`.
- **One monitor per session:** created at the phone's size (sharp) or half of it (fast), DPI set to the UI size
  (fast: half), placed right of all monitors (never primary, only that device is changed). The app window is moved
  onto it and maximised; the host captures the MONITOR, so menus, popups and dialogs show. Input maps to the monitor
  rectangle. Input is accepted while the foreground window belongs to the app (its process or owned by its window)
  and is on that monitor.
- **What is streamed:** the monitor's work area (the monitor minus the taskbar Explorer adds to every display, which
  appears some seconds after the monitor does; the host checks every 750 ms, follows a changed size or position and
  sends a new `window` message). The phone draws the video aspect-fit (black bars when the work area is a little
  shorter than the screen). Windows caps the DPI step by resolution (the smaller side divided by the scale stays at
  least 640 px, e.g. 225% at 1440x3120), so a higher `uiScale` is clamped; `window.dpi` and the settings readout show
  what was applied.
- **Keyboard:** the monitor stays. The window is restored and sized to the top rows and only those rows are
  streamed (the video is exactly the area above the keyboard, so the phone needs no cropping).
  Hiding the keyboard maximises the window again.
- **Rotation / quality change:** the monitor is switched to the new mode (if Windows accepts it, tested first) or
  removed and re-added under the same GUID; position, DPI and maximise are applied again.
- **Lifetime:** closing a tab removes the monitor at once; the window goes back to the primary monitor (the host
  moves it itself). A dropped WebSocket keeps the monitor 2 minutes; `open` with `attach` takes it over. Host exit
  (Ctrl+C, console close) removes all. Monitor GUIDs (marker `PENLINK!`, serial `PENLINK*****`) are listed in
  `%APPDATA%\Penlinkdisplays.json` and removed at the next start if the host crashed.

### Driver install
`penlink-host.exe` embeds the SudoVDA files (Apollo 0.4.6 release: `SudoVDA.dll/.inf`, `sudovda.cat`, `sudovda.cer`,
`nefconc.exe`; licences in `THIRD-PARTY-NOTICES.md`) and installs the driver itself, once, with one UAC prompt.

- **When:** at start, unless `--no-vdisplay` or `PENLINK_NO_VDISPLAY`: if the SudoVDA device cannot be opened and
  `driverDeclined` in `host.json` is not true.
- **How:** the host prints "Penlink needs to install its virtual display driver once; Windows will ask for
  permission.", unpacks the files to `%APPDATA%\Penlink\driver\<content hash>\`, and starts itself again with the
  `runas` verb as `penlink-host.exe --install-driver <dir>` (hidden window), waits, then looks for the device
  (up to 20 s). The elevated copy runs, in order (Apollo's script): `certutil -addstore -f root sudovda.cer`,
  `certutil -addstore -f TrustedPublisher sudovda.cer`, `nefconc --remove-device-node` (failure ignored),
  `nefconc --create-device-node --class-name Display ...`, `nefconc --install-driver --inf-path SudoVDA.inf`
  (no .bat). Output goes to `%APPDATA%\Penlink\driver-install.log`; the exit code is 0 on success, 1 otherwise.
- **Declined or failed:** one warning line, `driverDeclined: true` is saved in `host.json`, and the host runs with
  window capture. It does not ask again at later starts.
- **Flags:** `--install-driver-now` asks again (and clears the decline on success); `--uninstall-driver` removes the
  device node and both certificates (elevated, one UAC prompt). `--install-driver <dir>` and `--elevated-uninstall <dir>`
  are the internal elevated entry points.

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
- `{"type":"open","appId":"...","width":1440,"height":3120,"scale":2.0,"attach":false}`
  width/height = video size in pixels; scale = how much bigger the phone wants
  the UI than 100% (the host sizes the window to width/scale × height/scale
  and the video stays at width×height). The encoder limit is 3840×2160 (or 2160×3840): a larger
  request keeps its aspect ratio and the video is scaled down to fit that box.
  `attach` (optional, default false): do not launch the app, only take an existing window of it
  that no other session uses; none found → `closed` with reason `app exited`. Used when the phone
  re-opens its tabs after reconnecting. A window is never shared by two sessions.
  If the window does not fit the PC screen the host shrinks it, keeping the aspect ratio of the
  request, and still encodes the video at the requested pixel size (the picture is upscaled).
- `{"type":"answer","sessionId":"...","sdp":"..."}`
- `{"type":"ice","sessionId":"...","candidate":"...","sdpMid":"0","sdpMLineIndex":0}`
- `{"type":"resize","sessionId":"...","width":3120,"height":1440,"scale":2.0,"quality":"sharp","uiScale":300}`
  `quality` `"sharp"` (monitor = phone pixels) or `"fast"` (half, video upscaled on the phone) and `uiScale`
  (percent, 100-500; Windows DPI steps 100,125,150,175,200,225,250,300,350,400,450,500) are optional on `open`
  and `resize`. Without `uiScale`, `scale*100` is used; without `quality`, sharp. Sending either asks for a
  virtual monitor. Window capture honours them too (video halves in fast, window size stays).
- `{"type":"close","sessionId":"..."}`
- `{"type":"keyframe","sessionId":"..."}` ask for a new keyframe

Host → phone:
- `{"type":"opened","sessionId":"...","appId":"...","title":"..."}` sent **immediately**
  when the host accepts an `open` (before the app is launched). It assigns the
  sessionId; `title` may be empty. The phone shows a pending tab (spinner).
- `{"type":"title","sessionId":"...","title":"..."}` optional; the window title,
  once the window has been found (sent before `offer`).
- `{"type":"window","sessionId":"...","width":812,"height":1760,"fitted":true,"mode":"window"}` (monitor mode:
  `"mode":"monitor","dpi":300`, width/height = the streamed monitor area, `fitted` false, `dpi` = the step
  Windows really applied: the nearest allowed one if it refused the asked one). `mode` is missing from old hosts
  (= window). The real size of the
  PC window's client area in PC pixels, sent before `offer` and after every `resize`. `fitted` is true
  when the request was shrunk to fit the PC screen (so the video is upscaled and looks soft); the phone
  shows a hint in its settings sheet ("PC screen too small ... raise the UI scale": a higher scale
  makes the requested window smaller, so it fits).
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

Order for one `open`: `opened` → (`title`) → `window` → `offer` → `ice`*, or `opened` → `closed`.

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

**Keyboard: the phone's own keyboard plus one row of extra keys.** One keyboard button shows or hides
the system IME (through a hidden focused text sink). Committed text goes out as 	ext; Backspace,
Enter, Tab and Delete from the IME go out as key. A single row of extra keys is docked on top of
the IME and follows its animation: Fn (swaps the row to F1-F12), Esc, Tab, Ctrl, Alt, Shift, Win,
arrows, Home, End, PgUp, PgDn, Del. Modifiers are sticky: tap = next key only, tap again = locked,
tap again = released. While a modifier is down, characters typed in the IME are sent as real key
combos (key down/up with the US virtual-key of that character) instead of 	ext; Hebrew letters
map to the key at the same physical position on the standard Hebrew layout (ש = A, נ = B, ב = C ...),
so Ctrl+<ב> is Ctrl+C.

**Keyboard and window size.** While the IME and the extra row are visible the phone shows the video only
in the area above them and sends 
esize for that area (width = screen width, height = screen height minus
IME minus row), debounced (250 ms), so the PC window gets exactly that area. Hiding the keyboard restores
the full size.

**Reconnecting.** If the WebSocket drops (host restarted or crashed, network change) the phone keeps its
tabs in a "reconnecting" state, retries with backoff (0.5, 1, 2, 4, 5 s) and, once connected, re-opens
every tab with open + "attach":true (same appId, new session). After 30 s without success it shows
that the PC is unreachable with a Retry button (and keeps trying every 10 s). A tab whose app has exited
comes back as closed with reason pp exited and is removed.
## Done means
- `cargo build --release` and `cargo test` pass in `host/`.
- `./gradlew assembleDebug` and unit tests pass in `android/`.
- End to end on the Android emulator against the real host: pair, list apps,
  open Notepad, see it, type into it, click in it.
- Then on the real S26 Ultra over Tailscale.
