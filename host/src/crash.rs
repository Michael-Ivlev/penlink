//! Crash diagnostics: an unhandled-exception filter that writes a text report (exception
//! code, faulting address as `module+offset`, unwound stack) and a minidump to
//! `%APPDATA%\Penlink\crash\`. Added after an intermittent access violation: a fault offset
//! alone is hard to map back to code, a dump plus the PDB is not.

use std::ffi::c_void;
use std::io::Write;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::System::Diagnostics::Debug::{
    MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithThreadInfo, MiniDumpWriteDump, RtlLookupFunctionEntry,
    RtlVirtualUnwind, SetUnhandledExceptionFilter, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
    UNW_FLAG_NHANDLER,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Directory the reports go to: next to the token store.
pub fn default_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Penlink").join("crash")
}

/// Install the filter (idempotent). Call once at start-up.
pub fn install(dir: PathBuf) {
    if DIR.set(dir).is_ok() {
        unsafe {
            SetUnhandledExceptionFilter(Some(filter));
        }
    }
}

/// `module.exe+0x1234` for an address inside a loaded module.
fn describe(addr: usize) -> String {
    unsafe {
        let mut m = HMODULE::default();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(addr as *const u16),
            &mut m,
        )
        .is_err()
        {
            return format!("{addr:#x}");
        }
        let mut buf = [0u16; 512];
        let n = GetModuleFileNameW(Some(m), &mut buf) as usize;
        let path = String::from_utf16_lossy(&buf[..n]);
        let name = path.rsplit('\\').next().unwrap_or(&path);
        format!("{name}+{:#x}", addr - m.0 as usize)
    }
}

unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
    // Run only once even if several threads fault at the same time.
    static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if BUSY.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return 1; // EXCEPTION_EXECUTE_HANDLER: just die
    }
    let Some(dir) = DIR.get() else { return 0 };
    let _ = std::fs::create_dir_all(dir);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let base = dir.join(format!("crash-{stamp}-{}", unsafe { GetCurrentProcessId() }));
    let mut report = String::new();
    unsafe {
        let rec = &*(*info).ExceptionRecord;
        let ctx = (*info).ContextRecord;
        report += &format!("exception {:#x} at {}\n", rec.ExceptionCode.0, describe(rec.ExceptionAddress as usize));
        if rec.NumberParameters >= 2 {
            let kind = match rec.ExceptionInformation[0] {
                0 => "read",
                1 => "write",
                8 => "execute",
                _ => "?",
            };
            report += &format!("access violation: {kind} of {:#x}\n", rec.ExceptionInformation[1]);
        }
        report += &format!("thread {:?} (os id {})\n", std::thread::current().name(), GetCurrentThreadId());
        // Unwind the faulting thread's stack with the x64 unwind tables.
        let mut c = *ctx;
        report += "stack:\n";
        for i in 0..48 {
            let pc = c.Rip as usize;
            if pc == 0 {
                break;
            }
            report += &format!("  #{i:<2} {}\n", describe(pc));
            let mut image_base = 0u64;
            let fe = RtlLookupFunctionEntry(c.Rip, &mut image_base, None);
            if fe.is_null() {
                // leaf without unwind info: return address at [rsp]
                c.Rip = *(c.Rsp as *const u64);
                c.Rsp += 8;
                continue;
            }
            let mut handler_data: *mut c_void = std::ptr::null_mut();
            let mut establisher = 0u64;
            RtlVirtualUnwind(UNW_FLAG_NHANDLER, image_base, c.Rip, fe, &mut c, &mut handler_data, &mut establisher, None);
        }
        if let Ok(mut f) = std::fs::File::create(base.with_extension("txt")) {
            let _ = f.write_all(report.as_bytes());
        }
        eprintln!("\n*** penlink-host crashed ***\n{report}report: {}", base.with_extension("txt").display());
        if let Ok(f) = std::fs::File::create(base.with_extension("dmp")) {
            let mei = MINIDUMP_EXCEPTION_INFORMATION {
                ThreadId: GetCurrentThreadId(),
                ExceptionPointers: info as *mut EXCEPTION_POINTERS,
                ClientPointers: false.into(),
            };
            let _ = MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                HANDLE(f.as_raw_handle()),
                windows::Win32::System::Diagnostics::Debug::MINIDUMP_TYPE(
                    MiniDumpWithIndirectlyReferencedMemory.0 | MiniDumpWithThreadInfo.0,
                ),
                Some(&mei),
                None,
                None,
            );
        }
    }
    0 // EXCEPTION_CONTINUE_SEARCH: let Windows terminate the process (and WER log it)
}
