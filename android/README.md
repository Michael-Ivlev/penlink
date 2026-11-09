# Penlink Android app

Kotlin + Jetpack Compose phone app (package `dev.penlink`, minSdk 33, targetSdk 36).
It talks to `penlink-host` on your PC using the protocol in `../SPEC.md`.

## Build

Needs the Android Studio JDK (`JAVA_HOME`) and the Android SDK (platform 36). Put the SDK path in
`local.properties` (git-ignored), for example `sdk.dir=C:/Users/you/AppData/Local/Android/Sdk`.

```
gradlew.bat assembleDebug          # app/build/outputs/apk/debug/app-debug.apk
gradlew.bat testDebugUnitTest      # JVM unit tests (protocol JSON, coordinate/tilt/VK mapping)
```

Signed release build: put `storeFile`, `storePassword`, `keyAlias` and `keyPassword` in a properties file outside
the repo and point `PENLINK_SIGNING` (or the `penlink.signing` Gradle property) at it, then run
`gradlew.bat assembleRelease`. Without it the release APK is left unsigned.

Install on a connected phone or emulator: `adb install -r app\build\outputs\apk\debug\app-debug.apk`.

## Connect to a host

1. Start `penlink-host.exe` on the PC. It prints a 6-digit pairing code (valid 5 minutes).
2. In the app tap **Add PC**, enter the address and the code, tap **Pair**.
   - Real phone: the PC's Tailscale IP (100.x.y.z) or name. Port 47800 is added if you leave it out.
   - Android emulator: `10.0.2.2` (port 47800 is implied; `10.0.2.2:47800` also works).
   - LAN needs the host started with `--allow-lan`.
3. Tap the saved PC, then an app. It opens full screen; **+** at the top opens another app (each is a tab).

## In a session

The top strip hides itself after a few seconds; tap the small pill at the top edge to bring it back.
Strip buttons: new app, tabs (x closes), direct/trackpad mode, reset zoom, UI scale sheet, keyboard.

- S Pen: hover moves the cursor, tip is left click, side button right click, pressure and tilt are sent.
- Finger, direct mode: touch; long press = right click; two-finger drag = wheel; pinch = local zoom.
- Finger, trackpad mode: drag moves the cursor, tap = click, two-finger tap = right click,
  tap then drag = left-button drag, two-finger drag = wheel.
- Keyboard: the keyboard button in the strip shows/hides the phone's own keyboard; its text goes out as `text`
  (Backspace/Enter/Tab/Delete as keys). A row of extra keys sits on top of it: Fn (F1-F12), Esc, Tab,
  Ctrl/Alt/Shift/Win (tap = next key, tap again = locked, tap again = off), arrows, Home, End, PgUp, PgDn, Del;
  scroll the row sideways if it does not fit. With a modifier on, typed characters become real key combos
  (Hebrew letters map to the key at the same position on the Hebrew layout, so Ctrl+ב = Ctrl+C).
  While the keyboard is up the video only fills the area above it and the PC window is resized to that area.
- If the connection drops (host restart, network) the tabs show "Reconnecting" and come back by themselves
  (apps are re-attached, not relaunched; tabs whose app has exited are removed). After 30 s a message with
  a Retry button appears.
- Settings sheet: Quality (Sharp = phone resolution, Fast = half resolution upscaled) and UI size (100-400 %,
  Windows' DPI steps; default 300 %), remembered per PC. It also shows what the PC streams: "Virtual monitor"
  with the DPI Windows really applied, or "The app's window" when the PC has no SudoVDA driver (menus and
  dialogs outside the window then do not show). In window mode, if the PC screen is too small the sheet says
  so (a higher UI size makes the window smaller, so it fits).
- The session is edge to edge (cutout mode "always"): the video also covers the camera hole and the requested
  size is the whole display.

## Debug: open a raw appId

Debug builds only: `adb shell am start -n dev.penlink/.MainActivity --es openAppId "'exe:C:\path\testpad.exe C:\log.txt'"`
connects to the first saved PC and opens that appId in a new tab (used by the end-to-end test with the host's
`testpad` example window).
