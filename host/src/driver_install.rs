//! The "full package": penlink-host.exe carries the SudoVDA driver files and installs the
//! driver itself on first run, with one UAC prompt.
//!
//! Flow: the normal (non-elevated) host notices the driver is missing, unpacks the embedded files
//! to `%APPDATA%\Penlink\driver\<hash>\`, and starts itself again elevated (`runas`) as
//! `penlink-host.exe --install-driver <dir>`. That elevated copy runs the same steps as Apollo's
//! install script (certutil + nefconc), logs to `%APPDATA%\Penlink\driver-install.log` and exits
//! with a code. If the user says no to UAC, or the install fails, the host remembers it
//! (`driverDeclined` in host.json) and keeps working with window capture.
//!
//! SudoVDA: MIT, SudoMaker. nefconc: BSD-3-Clause, Nefarius. See THIRD-PARTY-NOTICES.md.

use crate::store::StoreData;
use std::path::{Path, PathBuf};

pub const SUDOVDA_DLL: &[u8] = include_bytes!("../driver/sudovda/SudoVDA.dll");
pub const SUDOVDA_INF: &[u8] = include_bytes!("../driver/sudovda/SudoVDA.inf");
pub const SUDOVDA_CAT: &[u8] = include_bytes!("../driver/sudovda/sudovda.cat");
pub const SUDOVDA_CER: &[u8] = include_bytes!("../driver/sudovda/sudovda.cer");
pub const NEFCONC_EXE: &[u8] = include_bytes!("../driver/sudovda/nefconc.exe");

/// The embedded files, by the name they get on disk.
pub fn embedded_files() -> [(&'static str, &'static [u8]); 5] {
    [
        ("SudoVDA.dll", SUDOVDA_DLL),
        ("SudoVDA.inf", SUDOVDA_INF),
        ("sudovda.cat", SUDOVDA_CAT),
        ("sudovda.cer", SUDOVDA_CER),
        ("nefconc.exe", NEFCONC_EXE),
    ]
}

pub const HARDWARE_ID: &str = r"root\sudomaker\sudovda";
pub const CLASS_GUID: &str = "4D36E968-E325-11CE-BFC1-08002BE10318";
/// Subject of the self-signed certificate that signs SudoVDA.dll and sudovda.cat.
pub const CERT_NAME: &str = "sudovda@su.mk";

/// FNV-1a 64 over file names and contents: names the extraction folder so a new driver version
/// (a new host build) lands in a new folder.
pub fn content_hash(files: &[(&str, &[u8])]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for (name, data) in files {
        feed(name.as_bytes());
        feed(&[0]);
        feed(&(data.len() as u64).to_le_bytes());
        feed(data);
    }
    format!("{h:016x}")
}

pub fn version_hash() -> String {
    content_hash(&embedded_files())
}

pub fn penlink_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("Penlink")
}

/// `<base>\driver\<hash>`
pub fn extract_dir(base: &Path, hash: &str) -> PathBuf {
    base.join("driver").join(hash)
}

pub fn log_path(base: &Path) -> PathBuf {
    base.join("driver-install.log")
}

/// Write the embedded files into `dir` (created if needed). Files that already have the right
/// size are kept.
pub fn extract_to(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, data) in embedded_files() {
        let p = dir.join(name);
        if std::fs::metadata(&p).map(|m| m.len() == data.len() as u64).unwrap_or(false) {
            continue;
        }
        std::fs::write(&p, data)?;
    }
    Ok(())
}

/// One external command: program and arguments.
pub type Step = (String, Vec<String>);

fn step(program: &str, args: &[&str]) -> Step {
    (program.to_string(), args.iter().map(|s| s.to_string()).collect())
}

fn remove_node_step() -> Step {
    step("nefconc.exe", &["--remove-device-node", "--hardware-id", HARDWARE_ID, "--class-guid", CLASS_GUID])
}

/// Apollo's install order. `required` false: a failure is logged and ignored (removing a node
/// that does not exist).
pub fn install_steps() -> Vec<(Step, bool)> {
    vec![
        (step("certutil", &["-addstore", "-f", "root", "sudovda.cer"]), true),
        (step("certutil", &["-addstore", "-f", "TrustedPublisher", "sudovda.cer"]), true),
        (remove_node_step(), false),
        (
            step(
                "nefconc.exe",
                &["--create-device-node", "--class-name", "Display", "--class-guid", CLASS_GUID, "--hardware-id", HARDWARE_ID],
            ),
            true,
        ),
        (step("nefconc.exe", &["--install-driver", "--inf-path", "SudoVDA.inf"]), true),
    ]
}

pub fn uninstall_steps() -> Vec<(Step, bool)> {
    vec![
        (remove_node_step(), true),
        (step("certutil", &["-delstore", "root", CERT_NAME]), false),
        (step("certutil", &["-delstore", "TrustedPublisher", CERT_NAME]), false),
    ]
}

/// Quote one argument for a Windows command line (CommandLineToArgvW rules).
pub fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Parameter string for the elevated copy: `--install-driver "<dir>"` (or the uninstall twin).
pub fn elevated_params(flag: &str, dir: &Path) -> String {
    format!("{flag} {}", quote_arg(&dir.to_string_lossy()))
}

// ---------------------------------------------------------------------------------------
// "Declined" memory in host.json (other keys are kept)

fn read_data(store_path: &Path) -> Option<StoreData> {
    std::fs::read_to_string(store_path).ok().and_then(|s| serde_json::from_str::<StoreData>(&s).ok())
}

pub fn load_declined(store_path: &Path) -> bool {
    read_data(store_path).is_some_and(|d| d.driver_declined)
}

pub fn save_declined(store_path: &Path, declined: bool) -> std::io::Result<()> {
    let mut data = read_data(store_path).unwrap_or_default();
    if data.driver_declined == declined && store_path.exists() {
        return Ok(());
    }
    data.driver_declined = declined;
    if let Some(dir) = store_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = store_path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&data).unwrap())?;
    std::fs::rename(&tmp, store_path)
}

/// What the startup check should do.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Plan {
    /// Driver present.
    Nothing,
    /// Missing, but the user said no before (run with --install-driver-now to ask again).
    SkipDeclined,
    Install,
}

pub fn plan(installed: bool, declined: bool, force: bool) -> Plan {
    if installed {
        Plan::Nothing
    } else if declined && !force {
        Plan::SkipDeclined
    } else {
        Plan::Install
    }
}

// ---------------------------------------------------------------------------------------
// Windows side

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::core::HRESULT;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const ERROR_CANCELLED: u32 = 1223;
    /// Exit codes of the elevated copy.
    pub const EXIT_OK: i32 = 0;
    pub const EXIT_FAILED: i32 = 1;

    #[derive(Debug)]
    pub enum Elevated {
        /// The elevated copy ran and exited with this code.
        Exited(u32),
        /// The user said no to the UAC prompt.
        Cancelled,
        /// Windows could not start it.
        Error(String),
    }

    /// Run this exe again as administrator with `params`, wait for it.
    pub fn run_elevated(params: &str) -> Elevated {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return Elevated::Error(format!("current_exe: {e}")),
        };
        let w = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let verb = w("runas");
        let file = w(&exe.to_string_lossy());
        let par = w(params);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(par.as_ptr()),
            nShow: 0, // SW_HIDE
            ..Default::default()
        };
        if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
            return if e.code() == HRESULT::from_win32(ERROR_CANCELLED) { Elevated::Cancelled } else { Elevated::Error(e.to_string()) };
        }
        let h = info.hProcess;
        if h.is_invalid() {
            return Elevated::Error("no process handle".into());
        }
        let mut code = 1u32;
        unsafe {
            if WaitForSingleObject(h, INFINITE) == WAIT_OBJECT_0 {
                let _ = GetExitCodeProcess(h, &mut code);
            }
            let _ = CloseHandle(h);
        }
        Elevated::Exited(code)
    }

    fn log_line(log: &mut Option<std::fs::File>, msg: &str) {
        println!("{msg}");
        if let Some(f) = log {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = writeln!(f, "[{secs}] {msg}");
        }
    }

    /// Run the steps in `dir` (the elevated side). Returns the process exit code.
    fn run_steps(steps: &[(Step, bool)], dir: &Path, log_file: &Path) -> i32 {
        if let Some(p) = log_file.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let mut log = std::fs::OpenOptions::new().create(true).append(true).open(log_file).ok();
        log_line(&mut log, &format!("--- Penlink driver step run in {}", dir.display()));
        let mut failed = false;
        for ((program, args), required) in steps {
            let shown = format!("{program} {}", args.join(" "));
            // nefconc.exe is looked up in `dir` (current_dir does not affect the program search
            // on its own), certutil in System32.
            let exe = if program.ends_with(".exe") { dir.join(program) } else { PathBuf::from(program) };
            let out = std::process::Command::new(exe).args(args).current_dir(dir).creation_flags(CREATE_NO_WINDOW).output();
            match out {
                Ok(o) => {
                    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                    let code = o.status.code().unwrap_or(-1);
                    log_line(&mut log, &format!("> {shown}  -> exit {code}"));
                    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                        if let Some(f) = &mut log {
                            let _ = writeln!(f, "    {l}");
                        }
                    }
                    if code != 0 && *required {
                        log_line(&mut log, &format!("step failed: {shown}"));
                        failed = true;
                        break;
                    }
                }
                Err(e) => {
                    log_line(&mut log, &format!("> {shown}  -> could not start: {e}"));
                    if *required {
                        failed = true;
                        break;
                    }
                }
            }
        }
        log_line(&mut log, if failed { "result: FAILED" } else { "result: ok" });
        if failed { EXIT_FAILED } else { EXIT_OK }
    }

    /// Elevated entry: install from `dir`.
    pub fn run_install(dir: &Path) -> i32 {
        run_steps(&install_steps(), dir, &log_path(&penlink_dir()))
    }

    /// Elevated entry: remove the device node and the certificates.
    pub fn run_uninstall(dir: &Path) -> i32 {
        run_steps(&uninstall_steps(), dir, &log_path(&penlink_dir()))
    }

    fn wait_installed(secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            if crate::vdisplay::is_installed() {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Startup check. Installs the driver (one UAC prompt) when it is missing, unless the user
    /// declined earlier and `force` is not set. Never fails the host: it just returns.
    pub fn ensure_driver(store_path: &Path, force: bool) {
        match plan(crate::vdisplay::is_installed(), load_declined(store_path), force) {
            Plan::Nothing => {}
            Plan::SkipDeclined => {
                tracing::info!("virtual display driver not installed (you declined earlier); run with --install-driver-now to be asked again");
            }
            Plan::Install => {
                if install_now(store_path) {
                    let _ = save_declined(store_path, false);
                }
            }
        }
    }

    /// Extract and run the elevated install. True when the driver is there afterwards.
    pub fn install_now(store_path: &Path) -> bool {
        let dir = extract_dir(&penlink_dir(), &version_hash());
        if let Err(e) = extract_to(&dir) {
            tracing::warn!("cannot unpack the driver files to {}: {e}; using window capture", dir.display());
            return false;
        }
        println!("Penlink needs to install its virtual display driver once; Windows will ask for permission.");
        let result = run_elevated(&elevated_params("--install-driver", &dir));
        let ok = match result {
            Elevated::Exited(0) => wait_installed(20),
            _ => false,
        };
        if ok {
            println!("Virtual display driver installed.");
            return true;
        }
        let why = match result {
            Elevated::Cancelled => "permission was declined".to_string(),
            Elevated::Error(e) => format!("could not start the installer: {e}"),
            Elevated::Exited(0) => "the driver did not appear after install".to_string(),
            Elevated::Exited(c) => format!("installer exited with code {c}, see {}", log_path(&penlink_dir()).display()),
        };
        tracing::warn!("virtual display driver not installed ({why}); using window capture. Run with --install-driver-now to try again");
        if let Err(e) = save_declined(store_path, true) {
            tracing::warn!("could not remember the decision in {}: {e}", store_path.display());
        }
        false
    }

    /// `--uninstall-driver`: elevated removal of the device node and certificates.
    pub fn uninstall_now() -> i32 {
        let dir = extract_dir(&penlink_dir(), &version_hash());
        if let Err(e) = extract_to(&dir) {
            eprintln!("cannot unpack the driver files to {}: {e}", dir.display());
            return 1;
        }
        println!("Removing the Penlink virtual display driver; Windows will ask for permission.");
        match run_elevated(&elevated_params("--elevated-uninstall", &dir)) {
            Elevated::Exited(0) => {
                println!("Driver removed.");
                0
            }
            Elevated::Exited(c) => {
                eprintln!("removal failed (exit {c}), see {}", log_path(&penlink_dir()).display());
                1
            }
            Elevated::Cancelled => {
                eprintln!("permission was declined; nothing changed");
                1
            }
            Elevated::Error(e) => {
                eprintln!("could not start the removal: {e}");
                1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("penlink-drv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn embedded_files_are_real() {
        for (name, data) in embedded_files() {
            assert!(data.len() > 500, "{name} looks empty");
        }
        assert_eq!(&NEFCONC_EXE[..2], b"MZ");
        assert_eq!(&SUDOVDA_DLL[..2], b"MZ");
    }

    #[test]
    fn hash_is_stable_and_content_sensitive() {
        let a = content_hash(&[("a", b"xyz"), ("b", b"1")]);
        assert_eq!(a, content_hash(&[("a", b"xyz"), ("b", b"1")]));
        assert_ne!(a, content_hash(&[("a", b"xyz"), ("b", b"2")]));
        assert_ne!(a, content_hash(&[("a", b"xy"), ("zb", b"1")]));
        assert_eq!(a.len(), 16);
        assert_eq!(version_hash().len(), 16);
    }

    #[test]
    fn extract_path_layout() {
        let base = Path::new(r"C:\Users\x\AppData\Roaming\Penlink");
        assert_eq!(extract_dir(base, "abcd"), base.join("driver").join("abcd"));
        assert_eq!(log_path(Path::new("base")), Path::new("base").join("driver-install.log"));
    }

    #[test]
    fn extraction_writes_all_files_and_is_repeatable() {
        let root = tmp("extract");
        let d = extract_dir(&root, &version_hash());
        extract_to(&d).unwrap();
        extract_to(&d).unwrap();
        for (name, data) in embedded_files() {
            assert_eq!(std::fs::read(d.join(name)).unwrap(), data);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn install_steps_follow_apollo_order() {
        let s = install_steps();
        let names: Vec<String> = s.iter().map(|((p, a), _)| format!("{p} {}", a[0])).collect();
        assert_eq!(
            names,
            ["certutil -addstore", "certutil -addstore", "nefconc.exe --remove-device-node", "nefconc.exe --create-device-node", "nefconc.exe --install-driver"]
        );
        assert_eq!(s[0].0.1, ["-addstore", "-f", "root", "sudovda.cer"]);
        assert_eq!(s[1].0.1[2], "TrustedPublisher");
        assert!(!s[2].1, "removing a missing node must not fail the install");
        assert!(s[3].1 && s[4].1);
        assert!(s[3].0.1.contains(&HARDWARE_ID.to_string()));
        assert_eq!(s[4].0.1, ["--install-driver", "--inf-path", "SudoVDA.inf"]);
    }

    #[test]
    fn uninstall_steps_remove_node_then_certs() {
        let s = uninstall_steps();
        assert_eq!(s[0].0.1[0], "--remove-device-node");
        assert_eq!(s[1].0.1, ["-delstore", "root", CERT_NAME]);
        assert_eq!(s[2].0.1, ["-delstore", "TrustedPublisher", CERT_NAME]);
    }

    #[test]
    fn quoting_follows_windows_rules() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg(r"C:\a b\c"), r#""C:\a b\c""#);
        assert_eq!(quote_arg(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(
            elevated_params("--install-driver", Path::new(r"C:\Users\Jo Doe\Penlink\driver\ab")),
            r#"--install-driver "C:\Users\Jo Doe\Penlink\driver\ab""#
        );
    }

    #[test]
    fn plan_decisions() {
        assert_eq!(plan(true, false, false), Plan::Nothing);
        assert_eq!(plan(true, true, true), Plan::Nothing);
        assert_eq!(plan(false, false, false), Plan::Install);
        assert_eq!(plan(false, true, false), Plan::SkipDeclined);
        assert_eq!(plan(false, true, true), Plan::Install);
    }

    #[test]
    fn declined_state_persists_and_keeps_tokens() {
        let d = tmp("declined");
        let p = d.join("host.json");
        assert!(!load_declined(&p));
        save_declined(&p, true).unwrap();
        assert!(load_declined(&p));
        // a store with tokens keeps them
        std::fs::write(&p, r#"{"tokens":[{"token":"t","deviceName":"phone","createdUnix":1}]}"#).unwrap();
        assert!(!load_declined(&p));
        save_declined(&p, true).unwrap();
        let data: StoreData = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(data.tokens.len(), 1);
        assert!(data.driver_declined);
        save_declined(&p, false).unwrap();
        assert!(!load_declined(&p));
        assert_eq!(serde_json::from_str::<StoreData>(&std::fs::read_to_string(&p).unwrap()).unwrap().tokens.len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }
}
