//! Launch an app and find its main top-level window.

use crate::apps::{activate_packaged, shell_launch, AppInfo, Launched};
use crate::winutil::{list_candidate_windows, process_exe_name, run_sta, Hwnd, WinInfo};
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Windows that belong to an open session. Two sessions never share one window: apps are found
/// by process name, so without this a second `open` of the same program could pick the window
/// the first session is already streaming (two capture pipelines and two input threads fighting
/// over one window).
static CLAIMED: Mutex<Option<HashSet<Hwnd>>> = Mutex::new(None);

/// Reservation of one window; released when dropped.
#[derive(Debug)]
pub struct Claim(Hwnd);

impl Claim {
    /// `None` when the window is already claimed.
    pub fn try_new(h: Hwnd) -> Option<Claim> {
        let mut g = CLAIMED.lock().unwrap();
        g.get_or_insert_with(HashSet::new).insert(h).then_some(Claim(h))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(set) = CLAIMED.lock().unwrap().as_mut() {
            set.remove(&self.0);
        }
    }
}

fn is_claimed(h: Hwnd) -> bool {
    CLAIMED.lock().unwrap().as_ref().is_some_and(|s| s.contains(&h))
}

/// How long `attach` waits for an already running app's window before giving up.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct Target {
    /// Path of the .lnk or exe handed to ShellExecute.
    pub file: String,
    /// Command-line arguments for `file` (only `exe:` ids carry any).
    pub args: String,
    /// Packaged app: launched through the activation manager instead of `file`.
    pub aumid: Option<String>,
    pub name: String,
    /// Lowercase exe file name expected for the app's process(es).
    pub exe_name: Option<String>,
}

impl Target {
    pub fn from_app(a: &AppInfo) -> Self {
        Self {
            file: a.lnk.to_string_lossy().to_string(),
            args: String::new(),
            aumid: a.aumid.clone(),
            name: a.name.clone(),
            exe_name: a.exe_name.clone(),
        }
    }

    /// Split the text after `exe:` into program and arguments. A quoted program may contain
    /// spaces; otherwise the program ends at the first `.exe` followed by a space.
    pub fn split_exe_command(cmd: &str) -> (String, String) {
        let cmd = cmd.trim();
        if let Some(rest) = cmd.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
            }
        }
        let lower = cmd.to_ascii_lowercase();
        if let Some(i) = lower.find(".exe ") {
            return (cmd[..i + 4].to_string(), cmd[i + 5..].trim().to_string());
        }
        (cmd.to_string(), String::new())
    }

    /// `exe:<path or file name> [arguments]` ids, for tests and tools.
    pub fn from_exe_id(id: &str) -> Option<Self> {
        let (path, args) = Self::split_exe_command(id.strip_prefix("exe:")?);
        let path = path.as_str();
        if path.is_empty() {
            return None;
        }
        let file_name = path.rsplit(['\\', '/']).next().unwrap_or(path).to_string();
        let stem = file_name.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
        let exe_name = if file_name.to_ascii_lowercase().ends_with(".exe") {
            file_name.to_ascii_lowercase()
        } else {
            format!("{}.exe", file_name.to_ascii_lowercase())
        };
        Some(Self { file: path.to_string(), args, aumid: None, name: stem, exe_name: Some(exe_name) })
    }
}

#[derive(Debug)]
pub struct Found {
    pub hwnd: Hwnd,
    pub title: String,
    pub pid: u32,
    pub claim: Claim,
}

/// Pure helper: does a window belong to the target app?
pub fn belongs(
    win_pid: u32,
    win_exe: Option<&str>,
    launched_pid: Option<u32>,
    target_exe: Option<&str>,
) -> bool {
    if launched_pid == Some(win_pid) {
        return true;
    }
    match (win_exe, target_exe) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// Pure helper: weaker fallback by title / exe stem for packaged apps whose process names
/// differ from the shortcut target.
pub fn name_matches(app_name: &str, win_title: &str, win_exe: Option<&str>) -> bool {
    let n = app_name.trim().to_lowercase();
    if n.len() < 3 {
        return false;
    }
    if win_title.to_lowercase().contains(&n) {
        return true;
    }
    if let Some(e) = win_exe {
        let stem = e.trim_end_matches(".exe").to_lowercase();
        let squashed: String = n.chars().filter(|c| c.is_alphanumeric()).collect();
        if stem.len() >= 3 && (squashed == stem || squashed.contains(&stem) || stem.contains(&squashed)) {
            return true;
        }
    }
    false
}

struct ExeCache(HashMap<u32, Option<String>>);

impl ExeCache {
    fn get(&mut self, pid: u32) -> Option<String> {
        self.0.entry(pid).or_insert_with(|| process_exe_name(pid)).clone()
    }
}

/// Blocking: launch and wait up to `timeout` for the app window. With `attach` the app is
/// not launched: only a window of an already running instance is taken (error `app exited`
/// when there is none), which is what a phone reconnecting after a host restart wants.
pub fn launch_and_find(t: &Target, timeout: Duration, attach: bool) -> Result<Found> {
    let t = t.clone();
    run_sta(move || launch_and_find_inner(&t, timeout, attach))
}

/// First window of the list that nobody else claimed yet, claimed for the caller. The
/// enumeration order is Z-order (topmost first), so that is the most recently activated one.
fn claim_first(v: Vec<(WinInfo, u32)>) -> Option<Found> {
    for (w, _) in v {
        if let Some(claim) = Claim::try_new(w.hwnd) {
            return Some(Found { hwnd: w.hwnd, title: w.title, pid: w.pid, claim });
        }
    }
    None
}

fn launch_and_find_inner(t: &Target, timeout: Duration, attach: bool) -> Result<Found> {
    let mut exes = ExeCache(HashMap::new());
    let before: HashSet<Hwnd> = list_candidate_windows().iter().map(|w| w.hwnd).collect();
    let started = Instant::now();
    let launched = if attach {
        Launched { pid: None }
    } else {
        match &t.aumid {
            Some(a) => activate_packaged(a)?,
            None => shell_launch(&t.file, &t.args)?,
        }
    };
    tracing::info!("{} {:?} (pid {:?}), waiting for its window", if attach { "attaching to" } else { "launched" }, t.file, launched.pid);

    let mut first_new: Option<Instant> = None;
    loop {
        let elapsed = started.elapsed();
        let wins = list_candidate_windows();
        let mut new_matches = Vec::new();
        let mut old_matches = Vec::new();
        for w in wins {
            if is_claimed(w.hwnd) {
                continue;
            }
            let exe = exes.get(w.pid);
            let strong = belongs(w.pid, exe.as_deref(), launched.pid, t.exe_name.as_deref());
            let weak = !strong && elapsed > Duration::from_millis(1500) && name_matches(&t.name, &w.title, exe.as_deref());
            if strong || weak {
                if before.contains(&w.hwnd) {
                    old_matches.push((w, 0));
                } else {
                    new_matches.push((w, 0));
                }
            }
        }
        if attach {
            // Existing windows only (a window the app opened a moment ago counts too).
            new_matches.extend(old_matches);
            if let Some(f) = claim_first(new_matches) {
                return Ok(f);
            }
            if elapsed >= ATTACH_TIMEOUT.min(timeout) {
                return Err(anyhow!("app exited"));
            }
        } else if !new_matches.is_empty() {
            let since = *first_new.get_or_insert_with(Instant::now);
            // let splash screens finish and the real window settle
            if since.elapsed() >= Duration::from_millis(400) || elapsed > timeout / 2 {
                if let Some(f) = claim_first(new_matches) {
                    return Ok(f);
                }
            }
        } else if elapsed >= Duration::from_millis(2500) && !old_matches.is_empty() {
            // Already running (single-instance app): use its existing window.
            if let Some(f) = claim_first(old_matches) {
                return Ok(f);
            }
        }
        if elapsed >= timeout {
            return Err(anyhow!("timed out waiting for the app window"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_ids() {
        let t = Target::from_exe_id("exe:notepad.exe").unwrap();
        assert_eq!(t.file, "notepad.exe");
        assert_eq!(t.name, "notepad");
        assert_eq!(t.exe_name.as_deref(), Some("notepad.exe"));
        let t = Target::from_exe_id(r"exe:C:\Windows\System32\Calc.EXE").unwrap();
        assert_eq!(t.exe_name.as_deref(), Some("calc.exe"));
        assert!(Target::from_exe_id("exe:").is_none());
        assert!(Target::from_exe_id("abc").is_none());
    }

    #[test]
    fn exe_ids_with_arguments() {
        let t = Target::from_exe_id(r"exe:C:\tools\testpad.exe C:\logs\a b.log").unwrap();
        assert_eq!(t.file, r"C:\tools\testpad.exe");
        assert_eq!(t.args, r"C:\logs\a b.log");
        assert_eq!(t.exe_name.as_deref(), Some("testpad.exe"));
        let t = Target::from_exe_id(r#"exe:"C:\Program Files\x\y.exe" --flag"#).unwrap();
        assert_eq!(t.file, r"C:\Program Files\x\y.exe");
        assert_eq!(t.args, "--flag");
        assert_eq!(t.exe_name.as_deref(), Some("y.exe"));
        assert_eq!(Target::from_exe_id("exe:notepad.exe").unwrap().args, "");
    }

    #[test]
    fn belongs_by_pid_or_exe() {
        assert!(belongs(10, None, Some(10), None));
        assert!(belongs(11, Some("Code.exe"), Some(10), Some("code.exe")));
        assert!(!belongs(11, Some("other.exe"), Some(10), Some("code.exe")));
        assert!(!belongs(11, None, Some(10), Some("code.exe")));
    }

    #[test]
    fn name_fallback() {
        assert!(name_matches("Calculator", "Calculator", Some("applicationframehost.exe")));
        assert!(name_matches("Windows Terminal", "pwsh", Some("windowsterminal.exe")));
        assert!(!name_matches("Foo", "Bar", Some("baz.exe")));
        assert!(!name_matches("ab", "abc", None));
    }
}
