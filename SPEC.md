# Penlink: spec

Run one PC app full screen on a Galaxy S26 Ultra, with or without the S Pen.
Windows first, Mac later. Used mainly for coding, plus general apps.

## What the user does
1. Opens Penlink on the phone and picks their PC (reached over Tailscale).
2. Sees a grid of the PC's installed apps and taps one.
3. The app opens on the phone full screen: only that app, not the PC desktop.
4. Opens more apps; each one is its own session. Rotating the phone resizes the app.

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
- **One monitor per session:** its size and DPI come from the phone's safe-area size and the UI size (see "UI size"
  below), placed right of all monitors (never primary, only that device is changed). The app window is moved
  onto it and maximised; the host captures the MONITOR, so menus, popups and dialogs show. Input maps to the monitor
  rectangle. Input is accepted while the foreground window belongs to the app (its process or owned by its window)
  and is on that monitor.
- **What is streamed:** the monitor's work area (the monitor minus the taskbar Explorer adds to every display, which
  appears some seconds after the monitor does; the host checks every 750 ms, follows a changed size or position and
  sends a new `window` message). The phone draws the video aspect-fit (black bars when the work area is a little
  shorter than the screen). `window.dpi` and the settings readout show the DPI that Windows really applied.
- **UI size (phone design):** `uiScale` is the UI size in percent. By default (`quality` `"fast"`, "Sharp text" off)
  the UI size is made by LOWER RESOLUTION: monitor = `width/uiScale*100` x `height/uiScale*100` (even numbers), Windows
  DPI 100%, and the phone upscales the video (200% on a 1440x3120 phone = a 720x1560 monitor; fewer pixels also encode
  faster). With "Sharp text" on (`quality` `"sharp"`) the monitor keeps the full size and Windows DPI scaling does the
  UI size; Windows caps it (the smaller side divided by the scale stays at least 640 px: 225% at 1440 px, 200% at
  1280 px), the nearest allowed step is used and reported in `window.dpi`. Measured with SudoVDA on Windows 11: the
  smallest monitor modes that become active are 200x433 and 520x240 (160x347 and 400x188 do not); the host never goes
  below 200 px on a side. Apps need room (the test app cannot be narrower than about 396 px), so the phone's slider
  keeps the smaller monitor side at 400 px or more: low-resolution range = 100%..`floor(smallerSide/400*4)*25`%
  in 25% steps (1440 px: 100..350%, 1280 px: 100..300%), sharp-text range = the Windows steps up to the cap above.
  Every step gives a different monitor size, so each one visibly changes the UI. Saved per PC as `v2:<fast|sharp>:<ui>`;
  older saved values (test values) become the default (low resolution, 200%).
- **Safe area:** the phone sends the size of its safe area (the window minus the display cutout and the system bars,
  both orientations) as `width`/`height`, draws the video and maps touch inside it and paints the rest black. The
  monitor therefore fills exactly the visible area.
- **Keyboard:** the monitor stays. The window is restored and sized to the top rows and only those rows are
  streamed (the video is exactly the area above the keyboard, so the phone needs no cropping).
  Hiding the keyboard maximises the window again.
- **Rotation / quality change:** the monitor is switched to the new mode (if Windows accepts it, tested first) or
  removed and re-added under the same GUID; position, DPI and maximise are applied again.
- **Lifetime:** closing a tab, leaving the app (back out of the PC, activity destroyed, app swiped away: the phone
  sends `close` for every tab and then a WebSocket close frame, from `onDestroy` and from a service's `onTaskRemoved`)
  removes the monitor at once and the window goes back to the primary monitor (the host moves it itself); measured:
  about 0.3 s on the host, under 0.5 s until the monitor is gone from the desktop. The host stops the capture, removes
  the monitor, and only then closes the peer connection. A broken connection (no close frame: network drop, phone
  killed) is noticed at once when the TCP connection ends, or after 12 s of silence otherwise (the host pings every
  4 s); the monitor is then kept 5 s (`GRACE`, checked every 0.5 s) so that a short blip re-attaches to it, after
  which it is removed and the window goes to the primary monitor; a phone that comes back later gets a new monitor.
  `open` with `attach` also takes over what its own old, not yet noticed as dead connection still holds. A removal
  that fails is retried every 0.5 s (up to 20 times). Host exit (Ctrl+C, console close) removes all. Monitor GUIDs (marker `PENLINK!`, serial `PENLINK*****`) are listed in
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

Host listens on **TCP 47800** (`PENLINK_VD_LEDGER` points the monitor ledger elsewhere for a second host used in tests), plain HTTP + WebSocket (Tailscale encrypts).
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
  width/height = the phone's safe-area size in pixels (display cutout and system bars excluded; for old clients: the video size); scale = how much bigger the phone wants
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
  `quality` `"fast"` (UI size by low resolution: monitor = size / uiScale at DPI 100%, the video is upscaled on the
  phone) or `"sharp"` (monitor = phone pixels, Windows DPI = uiScale, capped by Windows) and `uiScale`
  (percent, 100-800 accepted; the phone offers less, see "UI size") are optional on `open`
  and `resize`. Without `uiScale`, `scale*100` is used; without `quality`, sharp. Sending either asks for a
  virtual monitor. Window capture honours them too (fast: the window is video-sized, sharp: width/uiScale).
- `{"type":"close","sessionId":"..."}` closes one session; a WebSocket close frame (code 1000) means the phone left
  on purpose: every session ends now and the virtual monitors go at once. Any other end of the connection is a
  drop (see "Lifetime").
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

### Latency (input responsiveness)
- The host sends RTP header extension playout-delay min=max=0 on every video frame (the phone decodes and shows
  each frame at once, no smoothing buffer; measured on the emulator: jitter buffer 274 ms -> 0.8 ms) and opens a second
  data channel `pointer` (unordered, maxRetransmits 0). The phone sends pen/touch/mouse `move`/`hover` on it with an extra
  field `n` (counts up; the host drops a move older than one it already handled); everything else stays on `input`.
- `{"t":"ping","c":<phone µs>}` on `input` -> host answers `{"t":"pong","c":<same>,"h":<host µs>}` at once.
- Host -> phone `{"t":"lat","inj":..,"cap":..,"conv":..,"enc":..,"snd":..}` (µs, once per input event): input received ->
  injected, injected -> frame captured, captured -> encode start, encode, write to the track. The phone's debug overlay
  (off by default; Settings > Connection > "Show performance stats", saved; debug intent `--ez latency true|false`) adds RTT, jitter buffer and decode time from the WebRTC stats.
  `penlink-host --verbose` with `RUST_LOG=penlink_host::latency=debug` logs the same stages.
- Measuring: `examples/latprobe.rs` (loopback client) sends a mouse-down and times it until a decoded frame shows
  testpad's flash strip flip (testpad flips the bottom strip on every key, click and pointer-down).

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
2. **Apps:** an "Open now" section at the top lists the sessions that are already running (app icon, title, live
   state: Connected / Reconnecting... / Opening...; tap = resume at once, X = close it), then the grid of the host's
   apps with icons and a search box. Tapping a tile of an app that is already open resumes it instead of starting it
   again. Android Back here with apps open sends Penlink to the background (sessions stay); the arrow in the top bar
   leaves the PC (after a confirmation when apps are open: they are closed).
3. **Session:** only the app, full screen (immersive, no system bars, inside the safe area) plus the extra-keys row
   when the keyboard is up. No tab bar. A thin pill sits in the top strip (the black status-bar/cutout margin above the
   video, so it never covers the remote app); tap it or swipe down on it to open a control bar (auto-hides after 4 s):
   macOS-style round buttons at the left (red **Close**, yellow **Minimize**; 24 dp circles in 48 dp touch targets, glyph
   always visible), the app title, trackpad/direct toggle, reset zoom, session settings, keyboard. The red button
   is also the X on the rows of "Open now".
   - **Minimize** (and the Android Back gesture): back to the Apps screen. The session, the WebSocket, the PC app and
     its virtual monitor all stay, so resuming is instant. Only the phone's renderer is released while minimized (no
     video is drawn); the protocol has no pause message, so the host keeps streaming. The WebSocket lives as long as the
     process; if the link drops meanwhile the normal reconnect/attach path (below) brings the session back and "Open now"
     shows "Reconnecting...". If Android kills the process the sessions are gone (the host removes the monitor after the
     5 s grace); open the app again from the grid.
   - **Close**: sends `close`, the host stops the capture and removes the virtual monitor at once (about 0.3 s). The PC
     app itself is not touched: it keeps running, its window goes back to the primary monitor (same as closing a tab
     did before).

## Input on the phone
| | S Pen | Finger |
|---|---|---|
| Pointer | Hover moves the cursor without clicking | Direct mode (tap = click) or trackpad mode (relative cursor, tap = click) |
| Click | Tip = left click, side button = right click | Tap; long-press or two-finger tap = right click |
| Pressure / tilt | Sent as real Windows pen input | — |
| Scroll / zoom | — | Two-finger drag = wheel; pinch = zoom the phone's view (local) |

**Palm rejection (Settings > Pen > Palm rejection, on by default, saved for the whole app).** While the S Pen
writes or hovers, the resting hand must not click, scroll or zoom. The pen is "active" while it hovers or touches and for
600 ms after its last event (writers lift between strokes). While it is active every finger pointer is ignored, also in
events where the pen and the palm are down together (only the pen's pointer is read; the pen may even land after the
palm). A finger gesture already running when the pen arrives is cancelled at once (touch cancel / drag release sent, a
pinch or scroll ended, no click). Without a pen a contact longer than 150 dp, or one the system cancelled
(`FLAG_CANCELED`, API 33+), is a palm too: no tap is sent for it. After the grace period fingers work normally again.
Decisions are in `PalmRejector` (pure, unit tested); `InputOverlayView` applies them. With the setting off the old
behaviour returns. With a palm resting, a two-finger gesture is not recognised (a third pointer ends the gesture).

**Keyboard: the phone's own keyboard plus one row of extra keys.** One keyboard button shows or hides
the system IME (through a hidden focused text sink). Committed text goes out as 	ext; Backspace,
Enter, Tab and Delete from the IME go out as key. A single row of extra keys is docked on top of
the IME and follows its animation: Fn (swaps the row to F1-F12), Esc, Tab, Ctrl, Alt, Shift, Win,
arrows, Home, End, PgUp, PgDn, Del. Modifiers are sticky: tap = next key only, tap again = locked,
tap again = released. While a modifier is down, characters typed in the IME are sent as real key
combos (key down/up with the US virtual-key of that character) instead of 	ext; Hebrew letters
map to the key at the same physical position on the standard Hebrew layout (ש = A, נ = B, ב = C ...),
so Ctrl+<ב> is Ctrl+C.

**Keyboard and window size.** Default: the PC window and the video keep their size when the phone keyboard opens or
closes (no resize, no reflow, no flash); the keyboard simply overlays the bottom of the picture, like Windows App,
Jump Desktop and Splashtop do. The phone **pans** the picture up so the point you are working at stays visible:
- The focus point is the last press (finger, pen or mouse; in trackpad mode the cursor at the click). The protocol
  carries no caret position, so a tap in a text field is the best signal. While the keyboard is up a press only moves
  the focus further down, never back up, so tapping higher in the visible area does not make the picture jump.
- Pan amount (pure maths in `KeyboardPan`): 0 when the focus is already above the keyboard + extra-keys row (with a
  24 dp margin); otherwise just enough to bring it there, never more than aligning the bottom of the picture with the
  keyboard, and never so far that the focus leaves the top of the video area.
- It is driven by the IME inset, which Android animates frame by frame (WindowInsetsAnimation, read through Compose's
  `WindowInsets.ime`), so the picture moves with the keyboard and goes back to 0 as it hides. Only the view's
  translation changes; the SurfaceView is never resized or laid out. The extra-keys row slides up from behind the
  keyboard during the first row-height of the animation.
- It composes with the local pinch-zoom/pan: the displayed translation is user pan + keyboard pan, and `VideoMapper`
  (touch to video coordinates) is given both, so taps land exactly where they are drawn.
- Landscape: the keyboard covers most of the screen, so little of the picture stays visible. The focus is kept inside the
  visible strip (the top margin wins when the strip is too small for both margins), but the surrounding context is
  hidden. Hide the keyboard to see it, or turn on the setting below.
- Setting **"Resize app above keyboard"** (session settings, saved per host, default off) restores the old behaviour:
  the video area is the window minus keyboard minus extra row, and `resize` is sent (debounced 250 ms) for that area,
  so the PC window shrinks and the app reflows; hiding the keyboard restores the full size. No pan in this mode.
Rotation always sends one debounced `resize`, in both modes.

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
