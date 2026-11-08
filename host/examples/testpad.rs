//! Test target for end-to-end runs: a plain Win32 window "Penlink Testpad" with a multi-line
//! EDIT control. Every character, key, mouse and pen/touch pointer event it receives is
//! appended to the log file given as the first argument.
//!
//!     testpad.exe C:\path\to\testpad.log
//!
//! Open it through the host with the appId `exe:<full path>\testpad.exe <log path>`.

use std::io::Write;
use std::sync::OnceLock;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateFontW, GetStockObject, HBRUSH, WHITE_BRUSH, DEFAULT_CHARSET, FW_NORMAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::Pointer::{GetPointerPenInfo, GetPointerType, POINTER_PEN_INFO};
use windows::Win32::UI::WindowsAndMessaging::*;

static LOG: OnceLock<String> = OnceLock::new();
static mut OLD_EDIT_PROC: isize = 0;

fn log(line: &str) {
    if let Some(path) = LOG.get() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

unsafe extern "system" fn edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CHAR => {
            let c = char::from_u32(wp.0 as u32).unwrap_or('?');
            log(&format!("CHAR 0x{:02x} {:?}", wp.0, c));
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => log(&format!("KEYDOWN vk={}", wp.0)),
        WM_KEYUP | WM_SYSKEYUP => log(&format!("KEYUP vk={}", wp.0)),
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
            let (x, y) = xy(lp);
            let name = match msg {
                WM_LBUTTONDOWN => "LBUTTONDOWN",
                WM_RBUTTONDOWN => "RBUTTONDOWN",
                WM_MBUTTONDOWN => "MBUTTONDOWN",
                WM_LBUTTONUP => "LBUTTONUP",
                WM_RBUTTONUP => "RBUTTONUP",
                _ => "MBUTTONUP",
            };
            log(&format!("{name} x={x} y={y}"));
        }
        WM_MOUSEMOVE if wp.0 & 0x7 != 0 => {
            let (x, y) = xy(lp);
            log(&format!("DRAG x={x} y={y} keys=0x{:x}", wp.0));
        }
        WM_MOUSEWHEEL => log(&format!("WHEEL delta={}", (wp.0 >> 16) as i16)),
        WM_MOUSEHWHEEL => log(&format!("HWHEEL delta={}", (wp.0 >> 16) as i16)),
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            let id = (wp.0 & 0xffff) as u32;
            let mut ty = POINTER_INPUT_TYPE(0);
            let _ = unsafe { GetPointerType(id, &mut ty) };
            let name = match msg {
                WM_POINTERDOWN => "POINTERDOWN",
                WM_POINTERUP => "POINTERUP",
                _ => "POINTERUPDATE",
            };
            if ty == PT_PEN {
                let mut pi = POINTER_PEN_INFO::default();
                if unsafe { GetPointerPenInfo(id, &mut pi) }.is_ok() {
                    let p = pi.pointerInfo.ptPixelLocation;
                    log(&format!(
                        "{name} pen id={id} screen=({},{}) pressure={} tilt=({},{})",
                        p.x, p.y, pi.pressure, pi.tiltX, pi.tiltY
                    ));
                }
            } else if msg != WM_POINTERUPDATE {
                log(&format!("{name} type={}", ty.0));
            }
        }
        _ => {}
    }
    unsafe { CallWindowProcW(Some(std::mem::transmute::<isize, unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>(OLD_EDIT_PROC)), h, msg, wp, lp) }
}

static mut EDIT: HWND = HWND(std::ptr::null_mut());

unsafe extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            let mut r = RECT::default();
            let _ = unsafe { GetClientRect(h, &mut r) };
            log(&format!("SIZE client={}x{}", r.right, r.bottom));
            unsafe {
                let e = EDIT;
                if !e.0.is_null() {
                    let _ = MoveWindow(e, 0, 0, r.right, r.bottom, true);
                }
            }
            LRESULT(0)
        }
        WM_SETFOCUS | WM_ACTIVATE => {
            unsafe {
                let e = EDIT;
                if !e.0.is_null() {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(e));
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

fn main() {
    if let Some(p) = std::env::args().nth(1) {
        let _ = LOG.set(p);
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(PCWSTR::null()).unwrap();
        let class = w!("PenlinkTestpad");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: inst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Penlink Testpad"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            900,
            700,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .unwrap();
        let style = WS_CHILD | WS_VISIBLE | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32);
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            style,
            0,
            0,
            100,
            100,
            Some(hwnd),
            None,
            Some(inst.into()),
            None,
        )
        .unwrap();
        EDIT = edit;
        let font = CreateFontW(
            -44, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, Default::default(), Default::default(),
            Default::default(), Default::default(), w!("Consolas"),
        );
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        OLD_EDIT_PROC = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as usize as isize);
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let _ = MoveWindow(edit, 0, 0, r.right, r.bottom, true);
        log("START testpad");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        log("EXIT testpad");
    }
}
