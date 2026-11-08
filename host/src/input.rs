//! Input injection: SendInput for mouse/keyboard, synthetic pointer devices for pen/touch.
//!
//! The decision logic (which pointer flags to send for which phone event) is pure and unit
//! tested; the `Injector` applies it with Win32.

use crate::geom::{map_to_client, Rect};
use crate::sizing::{foreground_accepts, rect_center, rect_contains};
use crate::protocol::{Button, InputMsg, KeyAction, MouseAction, PenAction, TouchAction};
use crate::vk::{is_extended_key, us_char, VK_RETURN, VK_TAB};
use crate::winutil::{
    bring_to_front, client_rect_screen, foreground_info, is_foreground, is_window, last_active_popup, window_rect, Hwnd,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use windows::Win32::Foundation::{HANDLE, HWND, POINT, RECT};
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, DestroySyntheticPointerDevice, HSYNTHETICPOINTERDEVICE,
    POINTER_FEEDBACK_DEFAULT, POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    VIRTUAL_KEY,
};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAGS, POINTER_INFO, POINTER_PEN_INFO, POINTER_TOUCH_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, SetCursorPos, PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE,
    PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN, PT_TOUCH, TOUCH_FLAG_NONE, TOUCH_MASK_CONTACTAREA,
    TOUCH_MASK_PRESSURE,
};

// Raw POINTER_FLAG_* values (winuser.h).
pub const PF_NEW: u32 = 0x1;
pub const PF_INRANGE: u32 = 0x2;
pub const PF_INCONTACT: u32 = 0x4;
pub const PF_FIRSTBUTTON: u32 = 0x10;
pub const PF_SECONDBUTTON: u32 = 0x20;
pub const PF_PRIMARY: u32 = 0x2000;
pub const PF_CANCELED: u32 = 0x8000;
pub const PF_DOWN: u32 = 0x10000;
pub const PF_UPDATE: u32 = 0x20000;
pub const PF_UP: u32 = 0x40000;

// ---------------------------------------------------------------------------------------
// Pure pen logic

#[derive(Debug, Default, Clone)]
pub struct PenState {
    pub in_range: bool,
    pub contact: bool,
    /// Side button held when the tip went down: acting as a right mouse button.
    pub right_click: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PenStep {
    /// Inject a PT_PEN frame.
    Pointer { pointer_flags: u32, pen_flags: u32 },
    /// Move the mouse cursor to the pen position (right-click emulation).
    MoveCursor,
    RightDown,
    RightUp,
}

fn pen_bits(barrel: bool, eraser: bool, contact: bool) -> u32 {
    let mut f = 0;
    if barrel {
        f |= PEN_FLAG_BARREL;
    }
    if eraser {
        f |= PEN_FLAG_INVERTED;
        if contact {
            f |= PEN_FLAG_ERASER;
        }
    }
    f
}

pub fn pen_step(st: &mut PenState, a: PenAction, barrel: bool, eraser: bool) -> Vec<PenStep> {
    use PenAction::*;
    let mut out = Vec::new();
    let new = |st: &PenState| if st.in_range { 0 } else { PF_NEW };
    match a {
        Hover => {
            if st.right_click {
                out.push(PenStep::MoveCursor);
            } else if st.contact {
                // hover while we think we are touching: treat as lift
                out.push(PenStep::Pointer {
                    pointer_flags: PF_INRANGE | PF_UP | PF_PRIMARY,
                    pen_flags: pen_bits(barrel, eraser, false),
                });
                st.contact = false;
            } else {
                let flags = PF_INRANGE | PF_UPDATE | PF_PRIMARY | new(st) | if barrel { PF_SECONDBUTTON } else { 0 };
                out.push(PenStep::Pointer { pointer_flags: flags, pen_flags: pen_bits(barrel, eraser, false) });
                st.in_range = true;
            }
        }
        Down => {
            if st.right_click || st.contact {
                // duplicate down: treat as a move
                return pen_step(st, Move, barrel, eraser);
            }
            if barrel {
                st.right_click = true;
                out.push(PenStep::MoveCursor);
                out.push(PenStep::RightDown);
            } else {
                let flags = PF_INRANGE | PF_INCONTACT | PF_DOWN | PF_PRIMARY | PF_FIRSTBUTTON | new(st);
                out.push(PenStep::Pointer { pointer_flags: flags, pen_flags: pen_bits(false, eraser, true) });
                st.in_range = true;
                st.contact = true;
            }
        }
        Move => {
            if st.right_click {
                out.push(PenStep::MoveCursor);
            } else if st.contact {
                let flags = PF_INRANGE
                    | PF_INCONTACT
                    | PF_UPDATE
                    | PF_PRIMARY
                    | PF_FIRSTBUTTON
                    | if barrel { PF_SECONDBUTTON } else { 0 };
                out.push(PenStep::Pointer { pointer_flags: flags, pen_flags: pen_bits(barrel, eraser, true) });
            } else {
                return pen_step(st, Hover, barrel, eraser);
            }
        }
        Up => {
            if st.right_click {
                out.push(PenStep::RightUp);
                st.right_click = false;
            } else if st.contact {
                out.push(PenStep::Pointer {
                    pointer_flags: PF_INRANGE | PF_UP | PF_PRIMARY,
                    pen_flags: pen_bits(false, eraser, false),
                });
                st.contact = false;
            }
        }
        Leave => {
            if st.right_click {
                out.push(PenStep::RightUp);
                st.right_click = false;
            }
            if st.contact {
                out.push(PenStep::Pointer {
                    pointer_flags: PF_INRANGE | PF_UP | PF_PRIMARY,
                    pen_flags: pen_bits(false, eraser, false),
                });
                st.contact = false;
            }
            if st.in_range {
                // out of range: no INRANGE flag
                out.push(PenStep::Pointer { pointer_flags: PF_UPDATE | PF_PRIMARY, pen_flags: 0 });
                st.in_range = false;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Pure touch logic

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchPoint {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub flags: u32,
}

#[derive(Debug, Default)]
pub struct TouchState {
    active: BTreeMap<u32, (i32, i32)>,
}

impl TouchState {
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Returns the full frame to inject (every active contact must be in each call).
    pub fn apply(&mut self, a: TouchAction, id: u32, x: i32, y: i32) -> Vec<TouchPoint> {
        let base = PF_INRANGE | PF_INCONTACT;
        let mut frame = Vec::new();
        let mut finished = None;
        match a {
            TouchAction::Down => {
                // a second "down" for a live id is a move
                let flag = if self.active.contains_key(&id) { PF_UPDATE } else { PF_DOWN };
                self.active.insert(id, (x, y));
                for (&i, &(px, py)) in &self.active {
                    let f = if i == id { base | flag } else { base | PF_UPDATE };
                    frame.push(TouchPoint { id: i, x: px, y: py, flags: f });
                }
            }
            TouchAction::Move => {
                if !self.active.contains_key(&id) {
                    return frame;
                }
                self.active.insert(id, (x, y));
                for (&i, &(px, py)) in &self.active {
                    frame.push(TouchPoint { id: i, x: px, y: py, flags: base | PF_UPDATE });
                }
            }
            TouchAction::Up | TouchAction::Cancel => {
                if !self.active.contains_key(&id) {
                    return frame;
                }
                self.active.insert(id, (x, y));
                for (&i, &(px, py)) in &self.active {
                    let f = if i == id {
                        PF_INRANGE
                            | PF_UP
                            | if a == TouchAction::Cancel { PF_CANCELED } else { 0 }
                    } else {
                        base | PF_UPDATE
                    };
                    frame.push(TouchPoint { id: i, x: px, y: py, flags: f });
                }
                finished = Some(id);
            }
        }
        if let Some(id) = finished {
            self.active.remove(&id);
        }
        frame
    }

    /// Frame that lifts every contact (used on shutdown).
    pub fn cancel_all(&mut self) -> Vec<TouchPoint> {
        let frame = self
            .active
            .iter()
            .map(|(&i, &(x, y))| TouchPoint { id: i, x, y, flags: PF_INRANGE | PF_UP | PF_CANCELED })
            .collect();
        self.active.clear();
        frame
    }
}

// ---------------------------------------------------------------------------------------
// Win32 application

/// A virtual monitor shown to the phone: the app's window lives on it, and what the phone
/// sees is the monitor (or its top rows while the phone keyboard is up).
pub struct MonitorTarget {
    pub app_hwnd: Hwnd,
    pub app_pid: u32,
    /// The whole monitor, in screen coordinates.
    pub monitor: RwLock<Rect>,
    /// The streamed part of it, in screen coordinates.
    pub view: RwLock<Rect>,
}

impl MonitorTarget {
    pub fn new(app_hwnd: Hwnd, app_pid: u32, monitor: Rect) -> Self {
        Self { app_hwnd, app_pid, monitor: RwLock::new(monitor), view: RwLock::new(monitor) }
    }

    pub fn monitor_rect(&self) -> Rect {
        *self.monitor.read().unwrap()
    }

    pub fn view_rect(&self) -> Rect {
        *self.view.read().unwrap()
    }

    pub fn set(&self, monitor: Rect, view: Rect) {
        *self.monitor.write().unwrap() = monitor;
        *self.view.write().unwrap() = view;
    }

    /// The streamed part relative to the monitor's top-left (the capture crop).
    pub fn crop(&self) -> Rect {
        let (m, v) = (self.monitor_rect(), self.view_rect());
        Rect::new(v.x - m.x, v.y - m.y, v.w, v.h)
    }
}

/// What the phone sees and controls.
pub enum InputTarget {
    /// One window (its client area).
    Window(Hwnd),
    Monitor(MonitorTarget),
}

impl InputTarget {
    /// The app's main window.
    pub fn main_hwnd(&self) -> Hwnd {
        match self {
            InputTarget::Window(h) => *h,
            InputTarget::Monitor(m) => m.app_hwnd,
        }
    }

    /// The streamed area in screen coordinates.
    pub fn area(&self) -> Option<Rect> {
        match self {
            InputTarget::Window(h) => client_rect_screen(*h),
            InputTarget::Monitor(m) => Some(m.view_rect()),
        }
    }

    /// May input be injected right now? Window: the window is in front. Monitor: the
    /// foreground window belongs to the app (its process, or a dialog it owns) and is on the
    /// session's monitor, so a window the user switched to is never typed into.
    pub fn has_focus(&self) -> bool {
        match self {
            InputTarget::Window(h) => is_foreground(*h),
            InputTarget::Monitor(m) => {
                let Some(fg) = foreground_info() else { return false };
                let in_session = fg.pid == m.app_pid || fg.root_owner == m.app_hwnd;
                foreground_accepts(in_session, Some(fg.rect), m.monitor_rect())
            }
        }
    }

    /// Bring the app to the front (a dialog of it, if one is open).
    pub fn focus(&self) {
        match self {
            InputTarget::Window(h) => bring_to_front(*h),
            InputTarget::Monitor(m) => {
                // Only while the app is still on its monitor: never drag a window the user
                // moved away back into focus on the real desktop.
                if window_rect(m.app_hwnd).is_some_and(|r| rect_contains(m.monitor_rect(), rect_center(r))) {
                    bring_to_front(last_active_popup(m.app_hwnd));
                }
            }
        }
    }
}

/// State shared between the session, its capture pipeline and its input thread.
pub struct Shared {
    pub video_w: AtomicU32,
    pub video_h: AtomicU32,
    pub target: InputTarget,
    /// Latency records for the phone's debug overlay (see `latency`).
    pub lat_tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
}

impl Shared {
    pub fn new(w: u32, h: u32, target: InputTarget) -> Arc<Self> {
        Arc::new(Self { video_w: AtomicU32::new(w), video_h: AtomicU32::new(h), target, lat_tx: std::sync::Mutex::new(None) })
    }
    pub fn video(&self) -> (i32, i32) {
        (self.video_w.load(Ordering::Relaxed) as i32, self.video_h.load(Ordering::Relaxed) as i32)
    }
}

struct SafeDevice(HSYNTHETICPOINTERDEVICE);

impl Drop for SafeDevice {
    fn drop(&mut self) {
        unsafe { DestroySyntheticPointerDevice(self.0) };
    }
}

pub struct Injector {
    shared: Arc<Shared>,
    pen_dev: Option<SafeDevice>,
    touch_dev: Option<SafeDevice>,
    pen: PenState,
    touch: TouchState,
    held_keys: BTreeSet<u16>,
    /// Keys typed as Unicode on key-down (their key-up is swallowed).
    typed_keys: BTreeSet<u16>,
    held_buttons: BTreeSet<u8>, // 0 left, 1 right, 2 middle
    mrel_x: f64,
    mrel_y: f64,
    last_focus_try: std::time::Instant,
}

/// What a `mouse` message does: the cursor always goes to x,y first (absolute semantics);
/// a button press needs our window to have focus, a release is always sent.
#[derive(Debug, PartialEq, Eq)]
pub struct MousePlan {
    pub move_cursor: bool,
    pub send_button: bool,
}

pub fn mouse_plan(a: MouseAction, focused: bool) -> MousePlan {
    match a {
        MouseAction::Move => MousePlan { move_cursor: true, send_button: false },
        MouseAction::Down => MousePlan { move_cursor: focused, send_button: focused },
        MouseAction::Up => MousePlan { move_cursor: focused, send_button: true },
    }
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    let n = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if n as usize != inputs.len() {
        tracing::debug!("SendInput injected {n}/{}", inputs.len());
    }
}

impl Injector {
    pub fn new(shared: Arc<Shared>) -> Self {
        let pen_dev = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT) }
            .map(SafeDevice)
            .map_err(|e| tracing::warn!("pen injection unavailable: {e}"))
            .ok();
        let touch_dev = unsafe { CreateSyntheticPointerDevice(PT_TOUCH, 10, POINTER_FEEDBACK_DEFAULT) }
            .map(SafeDevice)
            .map_err(|e| tracing::warn!("touch injection unavailable: {e}"))
            .ok();
        Self {
            shared,
            pen_dev,
            touch_dev,
            pen: PenState::default(),
            touch: TouchState::default(),
            held_keys: BTreeSet::new(),
            typed_keys: BTreeSet::new(),
            held_buttons: BTreeSet::new(),
            mrel_x: 0.0,
            mrel_y: 0.0,
            last_focus_try: std::time::Instant::now() - std::time::Duration::from_secs(10),
        }
    }

    /// Make sure our window is the foreground window before injecting input. Returns false
    /// when it is not: SendInput goes to whatever has focus, so input is dropped rather than
    /// typed into some other app the user switched to.
    fn ensure_focus(&mut self) -> bool {
        if self.shared.target.has_focus() {
            return true;
        }
        // A press that finds the window not in front retries at once (a dropped press is a lost
        // tap); the limit only keeps a stream of moves from hammering SetForegroundWindow.
        if self.last_focus_try.elapsed() <= std::time::Duration::from_millis(40) {
            return false;
        }
        self.last_focus_try = std::time::Instant::now();
        self.shared.target.focus();
        for _ in 0..6 {
            if self.shared.target.has_focus() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        tracing::debug!("window {:#x} is not in the foreground; input dropped", self.shared.target.main_hwnd());
        false
    }

    /// Normalised video coords -> screen pixel position.
    fn to_screen(&self, nx: f64, ny: f64) -> Option<(i32, i32)> {
        let cr = self.shared.target.area()?;
        let (vw, vh) = self.shared.video();
        let (px, py) = map_to_client(nx, ny, vw, vh, cr.w, cr.h);
        Some((cr.x + px, cr.y + py))
    }

    pub fn handle(&mut self, msg: InputMsg) {
        tracing::trace!("input {msg:?}");
        if !is_window(self.shared.target.main_hwnd()) {
            return;
        }
        match msg {
            InputMsg::Pen { a, x, y, p, tx, ty, b, e } => self.pen(a, x, y, p, tx, ty, b & 1 != 0, e),
            InputMsg::Touch { a, id, x, y } => self.touch(a, id, x, y),
            InputMsg::Mouse { a, x, y, btn } => self.mouse(a, x, y, btn),
            InputMsg::Mrel { dx, dy } => self.mrel(dx, dy),
            InputMsg::Wheel { dx, dy } => self.wheel(dx, dy),
            InputMsg::Key { a, vk } => self.key(a, vk),
            InputMsg::Text { s } => self.text(&s),
            InputMsg::Ping { .. } => {}
        }
    }

    fn move_cursor(&mut self, nx: f64, ny: f64) {
        if let Some((sx, sy)) = self.to_screen(nx, ny) {
            unsafe {
                let _ = SetCursorPos(sx, sy);
            }
        }
    }

    fn mouse(&mut self, a: MouseAction, x: f64, y: f64, btn: Button) {
        let focused = self.ensure_focus();
        let plan = mouse_plan(a, focused);
        // Absolute semantics: the cursor goes to x,y before any button event, so a phone
        // trackpad (which keeps its own cursor) clicks where it shows its cursor.
        if plan.move_cursor {
            self.move_cursor(x, y);
        }
        let (down, up, id) = match btn {
            Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0u8),
            Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 1),
            Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 2),
        };
        match a {
            MouseAction::Move => {}
            MouseAction::Down => {
                if !plan.send_button {
                    return;
                }
                self.held_buttons.insert(id);
                send(&[mouse_input(down, 0)]);
            }
            MouseAction::Up => {
                self.held_buttons.remove(&id);
                send(&[mouse_input(up, 0)]);
            }
        }
    }

    fn mrel(&mut self, dx: f64, dy: f64) {
        let Some(cr) = self.shared.target.area() else { return };
        let (vw, vh) = self.shared.video();
        if vw <= 0 || vh <= 0 || cr.w <= 0 || cr.h <= 0 {
            return;
        }
        // video pixels -> client pixels
        let fit = crate::geom::aspect_fit(cr.w, cr.h, vw, vh);
        let k = if fit.w > 0 { cr.w as f64 / fit.w as f64 } else { 1.0 };
        self.mrel_x += dx * k;
        self.mrel_y += dy * k;
        let (ix, iy) = (self.mrel_x.trunc(), self.mrel_y.trunc());
        self.mrel_x -= ix;
        self.mrel_y -= iy;
        if ix == 0.0 && iy == 0.0 {
            return;
        }
        let mut p = POINT::default();
        unsafe {
            if GetCursorPos(&mut p).is_err() {
                return;
            }
            let nx = (p.x + ix as i32).clamp(cr.x, cr.x + cr.w - 1);
            let ny = (p.y + iy as i32).clamp(cr.y, cr.y + cr.h - 1);
            let _ = SetCursorPos(nx, ny);
        }
    }

    fn wheel(&mut self, dx: f64, dy: f64) {
        if !self.ensure_focus() {
            return;
        }
        // The wheel goes to the window under the cursor: make sure it is inside our client area.
        if let Some(cr) = self.shared.target.area() {
            let mut p = POINT::default();
            unsafe {
                if GetCursorPos(&mut p).is_ok()
                    && (p.x < cr.x || p.y < cr.y || p.x >= cr.x + cr.w || p.y >= cr.y + cr.h)
                {
                    let _ = SetCursorPos(cr.x + cr.w / 2, cr.y + cr.h / 2);
                }
            }
        }
        let mut v = Vec::new();
        if dy != 0.0 {
            v.push(mouse_input(MOUSEEVENTF_WHEEL, dy.round() as i32));
        }
        if dx != 0.0 {
            v.push(mouse_input(MOUSEEVENTF_HWHEEL, dx.round() as i32));
        }
        send(&v);
    }

    fn key(&mut self, a: KeyAction, vk: u16) {
        // Key-ups are always sent so a modifier never stays stuck; downs need focus.
        if !self.ensure_focus() && a == KeyAction::Down {
            return;
        }
        // Printable keys without Ctrl/Alt/Win: type the US-layout character as Unicode so the
        // PC's own layout (Hebrew, ...) cannot change what the phone keyboard meant.
        let mods = |v: u16| self.held_keys.contains(&v);
        let shortcut = mods(0x11) || mods(0xA2) || mods(0xA3) || mods(0x12) || mods(0xA4) || mods(0xA5) || mods(0x5B) || mods(0x5C);
        let shift = mods(0x10) || mods(0xA0) || mods(0xA1);
        match a {
            KeyAction::Down if !shortcut => {
                if let Some(c) = us_char(vk, shift) {
                    self.typed_keys.insert(vk);
                    let mut buf = [0u16; 2];
                    let mut v = Vec::new();
                    for u in c.encode_utf16(&mut buf) {
                        v.push(key_input(0, *u, KEYEVENTF_UNICODE));
                        v.push(key_input(0, *u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                    }
                    send(&v);
                    return;
                }
            }
            KeyAction::Up if self.typed_keys.remove(&vk) => return,
            _ => {}
        }
        let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if is_extended_key(vk) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        match a {
            KeyAction::Down => {
                self.held_keys.insert(vk);
            }
            KeyAction::Up => {
                self.held_keys.remove(&vk);
                flags |= KEYEVENTF_KEYUP;
            }
        }
        send(&[key_input(vk, scan, flags)]);
    }

    fn text(&mut self, s: &str) {
        if !self.ensure_focus() {
            return;
        }
        let mut v = Vec::new();
        for ch in s.chars() {
            match ch {
                '\r' => {}
                '\n' => {
                    let scan = unsafe { MapVirtualKeyW(VK_RETURN as u32, MAPVK_VK_TO_VSC) } as u16;
                    v.push(key_input(VK_RETURN, scan, KEYBD_EVENT_FLAGS(0)));
                    v.push(key_input(VK_RETURN, scan, KEYEVENTF_KEYUP));
                }
                '\t' => {
                    let scan = unsafe { MapVirtualKeyW(VK_TAB as u32, MAPVK_VK_TO_VSC) } as u16;
                    v.push(key_input(VK_TAB, scan, KEYBD_EVENT_FLAGS(0)));
                    v.push(key_input(VK_TAB, scan, KEYEVENTF_KEYUP));
                }
                c => {
                    let mut buf = [0u16; 2];
                    for u in c.encode_utf16(&mut buf) {
                        v.push(key_input(0, *u, KEYEVENTF_UNICODE));
                        v.push(key_input(0, *u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                    }
                }
            }
        }
        // Large bursts can make the target miss key-ups (stuck auto-repeat): feed in small
        // batches with a short pause.
        for chunk in v.chunks(16) {
            send(chunk);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pen(&mut self, a: PenAction, x: f64, y: f64, p: f64, tx: f64, ty: f64, barrel: bool, eraser: bool) {
        if !self.ensure_focus() {
            return;
        }
        let Some((sx, sy)) = self.to_screen(x, y) else { return };
        let steps = pen_step(&mut self.pen, a, barrel, eraser);
        for st in steps {
            match st {
                PenStep::MoveCursor => unsafe {
                    let _ = SetCursorPos(sx, sy);
                },
                PenStep::RightDown => send(&[mouse_input(MOUSEEVENTF_RIGHTDOWN, 0)]),
                PenStep::RightUp => send(&[mouse_input(MOUSEEVENTF_RIGHTUP, 0)]),
                PenStep::Pointer { pointer_flags, pen_flags } => {
                    let Some(dev) = &self.pen_dev else {
                        // fall back to plain mouse
                        unsafe {
                            let _ = SetCursorPos(sx, sy);
                        }
                        if pointer_flags & PF_DOWN != 0 {
                            send(&[mouse_input(MOUSEEVENTF_LEFTDOWN, 0)]);
                        } else if pointer_flags & PF_UP != 0 {
                            send(&[mouse_input(MOUSEEVENTF_LEFTUP, 0)]);
                        }
                        continue;
                    };
                    let info = POINTER_INFO {
                        pointerType: PT_PEN,
                        pointerId: 0,
                        pointerFlags: POINTER_FLAGS(pointer_flags),
                        ptPixelLocation: POINT { x: sx, y: sy },
                        ..Default::default()
                    };
                    let pen = POINTER_PEN_INFO {
                        pointerInfo: info,
                        penFlags: pen_flags,
                        penMask: PEN_MASK_PRESSURE | PEN_MASK_TILT_X | PEN_MASK_TILT_Y,
                        pressure: (p.clamp(0.0, 1.0) * 1024.0).round() as u32,
                        rotation: 0,
                        tiltX: tx.clamp(-90.0, 90.0).round() as i32,
                        tiltY: ty.clamp(-90.0, 90.0).round() as i32,
                    };
                    let ti = POINTER_TYPE_INFO { r#type: PT_PEN, Anonymous: POINTER_TYPE_INFO_0 { penInfo: pen } };
                    if let Err(e) = unsafe { InjectSyntheticPointerInput(dev.0, &[ti]) } {
                        tracing::debug!("pen inject failed: {e}");
                    }
                }
            }
        }
    }

    fn touch(&mut self, a: TouchAction, id: u32, x: f64, y: f64) {
        self.ensure_focus();
        let Some((sx, sy)) = self.to_screen(x, y) else { return };
        let frame = self.touch.apply(a, id, sx, sy);
        self.inject_touch(&frame);
    }

    fn inject_touch(&mut self, frame: &[TouchPoint]) {
        if frame.is_empty() {
            return;
        }
        let Some(dev) = &self.touch_dev else { return };
        let infos: Vec<POINTER_TYPE_INFO> = frame
            .iter()
            .map(|t| {
                let info = POINTER_INFO {
                    pointerType: PT_TOUCH,
                    pointerId: t.id,
                    pointerFlags: POINTER_FLAGS(t.flags),
                    ptPixelLocation: POINT { x: t.x, y: t.y },
                    ..Default::default()
                };
                let touch = POINTER_TOUCH_INFO {
                    pointerInfo: info,
                    touchFlags: TOUCH_FLAG_NONE,
                    touchMask: TOUCH_MASK_CONTACTAREA | TOUCH_MASK_PRESSURE,
                    rcContact: RECT { left: t.x - 2, top: t.y - 2, right: t.x + 2, bottom: t.y + 2 },
                    orientation: 90,
                    pressure: 512,
                    ..Default::default()
                };
                POINTER_TYPE_INFO { r#type: PT_TOUCH, Anonymous: POINTER_TYPE_INFO_0 { touchInfo: touch } }
            })
            .collect();
        if let Err(e) = unsafe { InjectSyntheticPointerInput(dev.0, &infos) } {
            tracing::debug!("touch inject failed: {e}");
        }
    }

    /// Release everything that might be held (keys, buttons, pen, touches).
    pub fn release_all(&mut self) {
        let keys: Vec<u16> = self.held_keys.iter().copied().collect();
        for vk in keys {
            self.key(KeyAction::Up, vk);
        }
        let mut v = Vec::new();
        for b in std::mem::take(&mut self.held_buttons) {
            v.push(mouse_input(
                match b {
                    0 => MOUSEEVENTF_LEFTUP,
                    1 => MOUSEEVENTF_RIGHTUP,
                    _ => MOUSEEVENTF_MIDDLEUP,
                },
                0,
            ));
        }
        if self.pen.right_click {
            v.push(mouse_input(MOUSEEVENTF_RIGHTUP, 0));
            self.pen.right_click = false;
        }
        send(&v);
        let frame = self.touch.cancel_all();
        self.inject_touch(&frame);
        if self.pen.contact || self.pen.in_range {
            if let Some(dev) = &self.pen_dev {
                let info = POINTER_INFO {
                    pointerType: PT_PEN,
                    pointerFlags: POINTER_FLAGS(PF_UPDATE | PF_PRIMARY),
                    ..Default::default()
                };
                let ti = POINTER_TYPE_INFO {
                    r#type: PT_PEN,
                    Anonymous: POINTER_TYPE_INFO_0 { penInfo: POINTER_PEN_INFO { pointerInfo: info, ..Default::default() } },
                };
                let _ = unsafe { InjectSyntheticPointerInput(dev.0, &[ti]) };
            }
            self.pen = PenState::default();
        }
    }
}

#[allow(dead_code)]
fn _unused(_: HANDLE, _: HWND) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(steps: &[PenStep]) -> Vec<u32> {
        steps
            .iter()
            .filter_map(|s| match s {
                PenStep::Pointer { pointer_flags, .. } => Some(*pointer_flags),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mouse_moves_cursor_before_buttons_and_never_clicks_unfocused() {
        assert_eq!(mouse_plan(MouseAction::Down, true), MousePlan { move_cursor: true, send_button: true });
        assert_eq!(mouse_plan(MouseAction::Up, true), MousePlan { move_cursor: true, send_button: true });
        assert_eq!(mouse_plan(MouseAction::Move, false), MousePlan { move_cursor: true, send_button: false });
        // not our window in front: no press, no cursor jump, but a release still goes out
        assert_eq!(mouse_plan(MouseAction::Down, false), MousePlan { move_cursor: false, send_button: false });
        assert!(mouse_plan(MouseAction::Up, false).send_button);
    }

    #[test]
    fn pen_hover_enters_range_with_new_flag_once() {
        let mut st = PenState::default();
        let s1 = pen_step(&mut st, PenAction::Hover, false, false);
        assert_eq!(flags(&s1), vec![PF_INRANGE | PF_UPDATE | PF_PRIMARY | PF_NEW]);
        let s2 = pen_step(&mut st, PenAction::Hover, false, false);
        assert_eq!(flags(&s2), vec![PF_INRANGE | PF_UPDATE | PF_PRIMARY]);
    }

    #[test]
    fn pen_down_move_up_sequence() {
        let mut st = PenState::default();
        pen_step(&mut st, PenAction::Hover, false, false);
        let d = pen_step(&mut st, PenAction::Down, false, false);
        assert_eq!(
            flags(&d),
            vec![PF_INRANGE | PF_INCONTACT | PF_DOWN | PF_PRIMARY | PF_FIRSTBUTTON]
        );
        let m = pen_step(&mut st, PenAction::Move, false, false);
        assert_eq!(
            flags(&m),
            vec![PF_INRANGE | PF_INCONTACT | PF_UPDATE | PF_PRIMARY | PF_FIRSTBUTTON]
        );
        let u = pen_step(&mut st, PenAction::Up, false, false);
        assert_eq!(flags(&u), vec![PF_INRANGE | PF_UP | PF_PRIMARY]);
        assert!(st.in_range && !st.contact);
        let l = pen_step(&mut st, PenAction::Leave, false, false);
        assert_eq!(flags(&l), vec![PF_UPDATE | PF_PRIMARY]);
        assert!(!st.in_range);
    }

    #[test]
    fn pen_down_without_hover_gets_new() {
        let mut st = PenState::default();
        let d = pen_step(&mut st, PenAction::Down, false, false);
        assert_eq!(flags(&d)[0] & PF_NEW, PF_NEW);
    }

    #[test]
    fn pen_barrel_on_tip_down_is_right_click() {
        let mut st = PenState::default();
        let d = pen_step(&mut st, PenAction::Down, true, false);
        assert_eq!(d, vec![PenStep::MoveCursor, PenStep::RightDown]);
        assert_eq!(pen_step(&mut st, PenAction::Move, true, false), vec![PenStep::MoveCursor]);
        assert_eq!(pen_step(&mut st, PenAction::Up, true, false), vec![PenStep::RightUp]);
        assert!(!st.right_click);
    }

    #[test]
    fn pen_barrel_hover_sets_barrel_flag() {
        let mut st = PenState::default();
        let s = pen_step(&mut st, PenAction::Hover, true, false);
        match &s[0] {
            PenStep::Pointer { pen_flags, .. } => assert_eq!(*pen_flags, PEN_FLAG_BARREL),
            _ => panic!(),
        }
    }

    #[test]
    fn pen_eraser_flags() {
        let mut st = PenState::default();
        let s = pen_step(&mut st, PenAction::Hover, false, true);
        match &s[0] {
            PenStep::Pointer { pen_flags, .. } => assert_eq!(*pen_flags, PEN_FLAG_INVERTED),
            _ => panic!(),
        }
        let s = pen_step(&mut st, PenAction::Down, false, true);
        match &s[0] {
            PenStep::Pointer { pen_flags, .. } => assert_eq!(*pen_flags, PEN_FLAG_INVERTED | PEN_FLAG_ERASER),
            _ => panic!(),
        }
    }

    #[test]
    fn pen_leave_while_touching_lifts_first() {
        let mut st = PenState::default();
        pen_step(&mut st, PenAction::Down, false, false);
        let l = pen_step(&mut st, PenAction::Leave, false, false);
        assert_eq!(flags(&l), vec![PF_INRANGE | PF_UP | PF_PRIMARY, PF_UPDATE | PF_PRIMARY]);
        assert!(!st.contact && !st.in_range);
    }

    #[test]
    fn touch_single_contact_lifecycle() {
        let mut t = TouchState::default();
        let f = t.apply(TouchAction::Down, 1, 10, 20);
        assert_eq!(f, vec![TouchPoint { id: 1, x: 10, y: 20, flags: PF_INRANGE | PF_INCONTACT | PF_DOWN }]);
        let f = t.apply(TouchAction::Move, 1, 11, 21);
        assert_eq!(f[0].flags, PF_INRANGE | PF_INCONTACT | PF_UPDATE);
        let f = t.apply(TouchAction::Up, 1, 11, 21);
        assert_eq!(f[0].flags, PF_INRANGE | PF_UP);
        assert!(t.is_empty());
        // move for an unknown id does nothing
        assert!(t.apply(TouchAction::Move, 1, 0, 0).is_empty());
    }

    #[test]
    fn touch_multitouch_frames_include_all_contacts() {
        let mut t = TouchState::default();
        t.apply(TouchAction::Down, 1, 10, 10);
        let f = t.apply(TouchAction::Down, 2, 50, 50);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].flags, PF_INRANGE | PF_INCONTACT | PF_UPDATE); // id 1 unchanged
        assert_eq!(f[1].flags, PF_INRANGE | PF_INCONTACT | PF_DOWN); // id 2 new
        let f = t.apply(TouchAction::Move, 2, 60, 60);
        assert_eq!(f.len(), 2);
        let f = t.apply(TouchAction::Cancel, 1, 10, 10);
        assert_eq!(f[0].flags, PF_INRANGE | PF_UP | PF_CANCELED);
        assert_eq!(f[1].flags, PF_INRANGE | PF_INCONTACT | PF_UPDATE);
        assert!(!t.is_empty());
        let f = t.cancel_all();
        assert_eq!(f.len(), 1);
        assert!(t.is_empty());
    }
}
