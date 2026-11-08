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
- Keyboard: Penlink's coding keyboard (modifiers: tap = next key, double tap = locked). "ABC" key at the
  right switches to the phone's normal keyboard, whose text goes out as `text` messages.
