//! Win32 helpers: window enumeration, geometry, foreground handling, process names, COM.

use crate::geom::{fit_within, Rect};
use anyhow::{anyhow, Result};
use std::ffi::c_void;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClientRect, GetForegroundWindow, GetMenu, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindow, IsWindowVisible, IsZoomed, SetForegroundWindow, SetWindowPos,
    ShowWindow, SystemParametersInfoW, GWL_EXSTYLE, GWL_STYLE, GW_OWNER, SPI_GETWORKAREA,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE,
    WINDOW_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

pub type Hwnd = isize;

pub fn hwnd(h: Hwnd) -> HWND {
    HWND(h as *mut c_void)
}

/// Make this process per-monitor-v2 DPI aware so all coordinates are physical pixels.
pub fn init_process() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    });
}

/// Run `f` on a fresh thread with a COM single-threaded apartment, and return its result.
pub fn run_sta<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(move || {
        let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let r = f();
        if init.is_ok() {
            unsafe { CoUninitialize() };
        }
        r
    })
    .join()
    .expect("sta thread panicked")
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

#[derive(Debug, Clone)]
pub struct WinInfo {
    pub hwnd: Hwnd,
    pub pid: u32,
    pub title: String,
}

pub fn window_title(h: Hwnd) -> String {
    unsafe {
        let w = hwnd(h);
        let n = GetWindowTextLengthW(w);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(w, &mut buf);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

pub fn is_window(h: Hwnd) -> bool {
    unsafe { IsWindow(Some(hwnd(h))).as_bool() }
}

pub fn window_pid(h: Hwnd) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd(h), Some(&mut pid)) };
    pid
}

fn is_cloaked(w: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        DwmGetWindowAttribute(w, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, 4).is_ok()
            && cloaked != 0
    }
}

/// Visible, titled, un-owned, non-tool top-level windows (candidates for "the app window").
pub fn list_candidate_windows() -> Vec<WinInfo> {
    unsafe extern "system" fn cb(w: HWND, lp: LPARAM) -> BOOL {
        let out = unsafe { &mut *(lp.0 as *mut Vec<WinInfo>) };
        unsafe {
            if !IsWindowVisible(w).as_bool() {
                return TRUE;
            }
            if GetWindow(w, GW_OWNER).map(|o| !o.0.is_null()).unwrap_or(false) {
                return TRUE;
            }
            let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(w, GWL_EXSTYLE) as u32);
            if ex.contains(WS_EX_TOOLWINDOW) || ex.contains(WS_EX_NOACTIVATE) {
                return TRUE;
            }
            if is_cloaked(w) {
                return TRUE;
            }
            let mut r = RECT::default();
            if GetWindowRect(w, &mut r).is_err() || (r.right - r.left) < 50 || (r.bottom - r.top) < 50 {
                return TRUE;
            }
            let h = w.0 as isize;
            let title = window_title(h);
            if title.is_empty() {
                return TRUE;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(w, Some(&mut pid));
            if pid == GetCurrentProcessId() {
                return TRUE;
            }
            out.push(WinInfo { hwnd: h, pid, title });
        }
        TRUE
    }
    let mut out: Vec<WinInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut Vec<WinInfo> as isize));
    }
    out
}

/// Lowercase file name of the process image, e.g. `notepad.exe`.
pub fn process_exe_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(|s| s.to_ascii_lowercase())
    }
}

fn rect_from(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// Client area in screen coordinates (physical pixels).
pub fn client_rect_screen(h: Hwnd) -> Option<Rect> {
    unsafe {
        let w = hwnd(h);
        let mut r = RECT::default();
        GetClientRect(w, &mut r).ok()?;
        let mut p = POINT { x: 0, y: 0 };
        if !ClientToScreen(w, &mut p).as_bool() {
            return None;
        }
        Some(Rect::new(p.x, p.y, r.right - r.left, r.bottom - r.top))
    }
}

/// Visible frame bounds (without the invisible resize border / shadow) in screen coordinates.
pub fn frame_bounds(h: Hwnd) -> Option<Rect> {
    unsafe {
        let w = hwnd(h);
        let mut r = RECT::default();
        if DwmGetWindowAttribute(
            w,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
        {
            return Some(rect_from(r));
        }
        GetWindowRect(w, &mut r).ok()?;
        Some(rect_from(r))
    }
}

pub fn is_minimized(h: Hwnd) -> bool {
    unsafe { IsIconic(hwnd(h)).as_bool() }
}

pub fn primary_work_area() -> Rect {
    let mut r = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    if r.right <= r.left {
        Rect::new(0, 0, 1280, 720)
    } else {
        rect_from(r)
    }
}

/// Restore the window if minimised/maximised, then size it so its client area is
/// `cw` x `ch` (shrunk to fit the primary monitor, aspect kept) and put it on the primary
/// monitor. Returns the achieved client size. Blocking (synchronous SetWindowPos).
pub fn resize_client(h: Hwnd, cw: i32, ch: i32) -> Result<Resized> {
    unsafe {
        let w = hwnd(h);
        if !IsWindow(Some(w)).as_bool() {
            return Err(anyhow!("window is gone"));
        }
        if IsIconic(w).as_bool() || IsZoomed(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let work = primary_work_area();
        let style = WINDOW_STYLE(GetWindowLongPtrW(w, GWL_STYLE) as u32);
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(w, GWL_EXSTYLE) as u32);
        let has_menu = GetMenu(w).0 != std::ptr::null_mut();
        let dpi = match GetDpiForWindow(w) {
            0 => 96,
            d => d,
        };
        let extra = |cw: i32, ch: i32| -> (i32, i32) {
            let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
            if AdjustWindowRectExForDpi(&mut r, style, has_menu, ex, dpi).is_err() {
                return (0, 0);
            }
            ((r.right - r.left) - cw, (r.bottom - r.top) - ch)
        };
        let (ew, eh) = extra(cw, ch);
        let wanted = (cw, ch);
        // Too big for the PC screen: shrink, keeping the aspect ratio of the request. The video
        // stays at the requested pixel size (the phone's picture is then upscaled).
        let (cw, ch) = fit_within(cw, ch, (work.w - ew).max(1), (work.h - eh).max(1));
        let (mut ow, mut oh) = (cw + ew, ch + eh);
        let (x, y) = (work.x, work.y);
        let flags = SWP_NOZORDER | SWP_NOACTIVATE;
        let mut got = (0, 0);
        for _ in 0..4 {
            SetWindowPos(w, None, x, y, ow, oh, flags).map_err(|e| anyhow!("SetWindowPos: {e}"))?;
            let mut r = RECT::default();
            GetClientRect(w, &mut r).map_err(|e| anyhow!("GetClientRect: {e}"))?;
            got = (r.right - r.left, r.bottom - r.top);
            if got == (cw, ch) {
                break;
            }
            // Apps (and the invisible resize borders) can make the real frame differ from
            // AdjustWindowRect's idea: correct by the observed difference.
            let (dw, dh) = (cw - got.0, ch - got.1);
            if dw == 0 && dh == 0 {
                break;
            }
            ow += dw;
            oh += dh;
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        Ok(Resized { wanted, got, fitted: (cw, ch) != wanted })
    }
}

/// Outcome of [`resize_client`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resized {
    /// Client size asked for.
    pub wanted: (i32, i32),
    /// Client size the window really has.
    pub got: (i32, i32),
    /// The request did not fit the PC screen and was shrunk (aspect ratio kept).
    pub fitted: bool,
}

/// Bring a window to the foreground even though we are a background process.
pub fn bring_to_front(h: Hwnd) {
    unsafe {
        let w = hwnd(h);
        if GetForegroundWindow() == w {
            return;
        }
        if IsIconic(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
        }
        let fg = GetForegroundWindow();
        let fg_tid = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let me = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != me && AttachThreadInput(me, fg_tid, true).as_bool();
        let _ = BringWindowToTop(w);
        let ok = SetForegroundWindow(w).as_bool();
        if attached {
            let _ = AttachThreadInput(me, fg_tid, false);
        }
        if !ok {
            tracing::debug!("SetForegroundWindow refused for {h:#x}");
        }
    }
}

pub fn is_foreground(h: Hwnd) -> bool {
    unsafe { GetForegroundWindow() == hwnd(h) }
}
