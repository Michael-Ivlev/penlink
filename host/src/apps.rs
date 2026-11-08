//! Installed app list from Start Menu shortcuts, app icons, and launching.

use crate::protocol::AppEntry;
use crate::winutil::{from_wide, run_sta, wide};
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, SIZE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, STGM_READ,
};
use windows::Win32::System::Threading::GetProcessId;
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, IApplicationActivationManager, IEnumShellItems, IShellItem, IShellItemImageFactory,
    IShellLinkW, SHCreateItemFromParsingName, SHGetKnownFolderItem, SHGetKnownFolderPath, ShellExecuteExW,
    BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_CommonPrograms, FOLDERID_Programs, KF_FLAG_DEFAULT,
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY, ShellLink, AO_NONE,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub const ICON_SIZE: usize = 96;

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub lnk: PathBuf,
    /// Resolved target (may be empty for packaged apps).
    pub target: String,
    /// Lowercase exe file name of the target, if any.
    pub exe_name: Option<String>,
    /// Application User Model ID for apps listed in `shell:AppsFolder` (packaged / Store apps).
    /// These have no `.lnk`; `lnk` is empty.
    pub aumid: Option<String>,
}

impl AppInfo {
    pub fn entry(&self) -> AppEntry {
        AppEntry { id: self.id.clone(), name: self.name.clone() }
    }
}

/// FNV-1a 64-bit of the lower-cased path -> 16 hex chars. Stable across runs.
pub fn stable_id(path: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.to_lowercase().replace('/', "\\").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Skip uninstallers, docs, help, readme-style shortcuts.
pub fn should_skip(name: &str, target: &str) -> bool {
    let n = name.to_lowercase();
    const BAD_WORDS: &[&str] = &[
        "uninstall",
        "uninst",
        "remove ",
        "readme",
        "read me",
        "release notes",
        "documentation",
        "user guide",
        "user manual",
        "manual",
        "license",
        "eula",
        "changelog",
        "what's new",
        "whats new",
        "website",
        "web site",
        "help",
        "faq",
        "support",
        "online ",
        "visit ",
        "get started",
        "tutorial",
    ];
    if BAD_WORDS.iter().any(|w| n.contains(w)) {
        return true;
    }
    let t = target.to_lowercase();
    let base = t.rsplit(['\\', '/']).next().unwrap_or("");
    if base.starts_with("unins") || base.contains("uninstall") || base == "msiexec.exe" {
        return true;
    }
    if !t.is_empty() {
        let ext = base.rsplit('.').next().unwrap_or("");
        // Only things we can actually run as an app window.
        if !matches!(ext, "exe" | "bat" | "cmd" | "com" | "msc" | "cpl") {
            return true;
        }
    }
    false
}

/// Sort by name (case-insensitive) and drop duplicates (same display name, or same lnk id).
pub fn sort_dedupe(mut apps: Vec<AppInfo>) -> Vec<AppInfo> {
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            // a Start Menu shortcut wins over an AppsFolder entry of the same name
            .then_with(|| a.aumid.is_some().cmp(&b.aumid.is_some()))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut seen_names = HashSet::new();
    let mut seen_ids = HashSet::new();
    apps.retain(|a| seen_ids.insert(a.id.clone()) && seen_names.insert(a.name.to_lowercase()));
    apps
}

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn collect_lnks(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 4 {
                collect_lnks(&p, depth + 1, out);
            }
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
            out.push(p);
        }
    }
}

fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(j) = after.find('%') {
            let var = &after[..j];
            match std::env::var(var) {
                Ok(v) => out.push_str(&v),
                Err(_) => {
                    out.push('%');
                    out.push_str(var);
                    out.push('%');
                }
            }
            rest = &after[j + 1..];
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Resolve target path of a .lnk. Must be called on a COM-initialised thread.
fn resolve_lnk(path: &Path) -> Result<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let pf: IPersistFile = link.cast()?;
        let w = wide(&path.to_string_lossy());
        pf.Load(PCWSTR(w.as_ptr()), STGM_READ)?;
        let mut buf = [0u16; 1024];
        let mut fd = WIN32_FIND_DATAW::default();
        // Ignore failure: packaged-app shortcuts have no filesystem target.
        let _ = link.GetPath(&mut buf, &mut fd, 0);
        Ok(expand_env(&from_wide(&buf)))
    }
}

/// Enumerate Start Menu shortcuts. Blocking; creates its own COM apartment.
pub fn scan_apps() -> Vec<AppInfo> {
    run_sta(|| {
        let mut files = Vec::new();
        for id in [&FOLDERID_Programs, &FOLDERID_CommonPrograms] {
            if let Some(dir) = known_folder(id) {
                collect_lnks(&dir, 0, &mut files);
            }
        }
        let mut apps = Vec::new();
        for f in files {
            let name = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let target = resolve_lnk(&f).unwrap_or_default();
            if should_skip(&name, &target) {
                continue;
            }
            let exe_name = Path::new(&target)
                .file_name()
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                .filter(|s| s.ends_with(".exe"));
            apps.push(AppInfo {
                id: stable_id(&f.to_string_lossy()),
                name,
                lnk: f,
                target,
                exe_name,
                aumid: None,
            });
        }
        match scan_apps_folder() {
            Ok(v) => apps.extend(v),
            Err(e) => tracing::warn!("shell:AppsFolder enumeration failed: {e}"),
        }
        sort_dedupe(apps)
    })
}

/// Stable id of an AppsFolder entry.
pub fn packaged_id(aumid: &str) -> String {
    stable_id(&format!("appsfolder:{aumid}"))
}

/// `true` for AppsFolder parsing names we can launch by AUMID. Desktop entries show up there
/// too, as filesystem paths or `{KNOWNFOLDER-GUID}\...` (the Start Menu shortcuts cover those).
pub fn is_aumid(parsing_name: &str) -> bool {
    !parsing_name.is_empty()
        && !parsing_name.starts_with('{')
        && !parsing_name.contains('\\')
        && !parsing_name.contains('/')
        && !parsing_name.contains(':')
}

unsafe fn display_name(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    let p = unsafe { item.GetDisplayName(kind) }.ok()?;
    let s = unsafe { p.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

/// Packaged / Store apps (Terminal, Calculator, new Notepad, ...) from `shell:AppsFolder`.
/// Must run on a COM-initialised (STA) thread. Never launches anything.
pub fn scan_apps_folder() -> Result<Vec<AppInfo>> {
    let mut out = Vec::new();
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, Some(HANDLE::default()))?;
        let en: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut items: [Option<IShellItem>; 1] = [None];
            let mut fetched = 0u32;
            let _ = en.Next(&mut items, Some(&mut fetched));
            if fetched == 0 {
                break;
            }
            let Some(item) = items[0].take() else { break };
            let (Some(name), Some(aumid)) =
                (display_name(&item, SIGDN_NORMALDISPLAY), display_name(&item, SIGDN_PARENTRELATIVEPARSING))
            else {
                continue;
            };
            if name.trim().is_empty() || !is_aumid(&aumid) || should_skip(&name, "") || name.trim().eq_ignore_ascii_case("remove") {
                continue;
            }
            out.push(AppInfo {
                id: packaged_id(&aumid),
                name: name.trim().to_string(),
                lnk: PathBuf::new(),
                target: String::new(),
                exe_name: None,
                aumid: Some(aumid),
            });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------
// Icons

/// Nearest-neighbour fit of an RGBA image into a `size` x `size` canvas (aspect kept,
/// centred, transparent padding).
pub fn fit_rgba_square(src: &[u8], w: usize, h: usize, size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size * size * 4];
    if w == 0 || h == 0 {
        return out;
    }
    if w == size && h == size {
        out.copy_from_slice(&src[..size * size * 4]);
        return out;
    }
    let scale = (size as f64 / w as f64).min(size as f64 / h as f64);
    let (nw, nh) = (((w as f64 * scale).round() as usize).clamp(1, size), ((h as f64 * scale).round() as usize).clamp(1, size));
    let (ox, oy) = ((size - nw) / 2, (size - nh) / 2);
    for y in 0..nh {
        let sy = (y * h / nh).min(h - 1);
        for x in 0..nw {
            let sx = (x * w / nw).min(w - 1);
            let s = (sy * w + sx) * 4;
            let d = ((oy + y) * size + ox + x) * 4;
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}

pub fn encode_png_rgba(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header()?;
        wr.write_image_data(rgba)?;
    }
    Ok(out)
}

/// BGRA premultiplied -> RGBA straight.
pub fn bgra_premul_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let any_alpha = bgra.chunks_exact(4).any(|p| p[3] != 0);
    let mut out = Vec::with_capacity(bgra.len());
    for p in bgra.chunks_exact(4) {
        let a = if any_alpha { p[3] as u32 } else { 255 };
        let un = |c: u8| -> u8 {
            if !any_alpha || a == 255 || a == 0 {
                c
            } else {
                ((c as u32 * 255 + a / 2) / a).min(255) as u8
            }
        };
        out.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0]), a as u8]);
    }
    out
}

fn icon_for_path(path: &Path) -> Result<Vec<u8>> {
    unsafe {
        let w = wide(&path.to_string_lossy());
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None)?;
        let hbmp = factory.GetImage(SIZE { cx: ICON_SIZE as i32, cy: ICON_SIZE as i32 }, SIIGBF_ICONONLY)?;
        let hdc = GetDC(None);
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        // First call fills width/height.
        let _ = GetDIBits(hdc, hbmp, 0, 0, None, &mut bmi, DIB_RGB_COLORS);
        let (bw, bh) = (bmi.bmiHeader.biWidth, bmi.bmiHeader.biHeight.abs());
        if bw <= 0 || bh <= 0 {
            let _ = ReleaseDC(None, hdc);
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
            return Err(anyhow!("empty icon bitmap"));
        }
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;
        bmi.bmiHeader.biHeight = -bh; // top-down
        let mut buf = vec![0u8; (bw * bh * 4) as usize];
        let lines = GetDIBits(hdc, hbmp, 0, bh as u32, Some(buf.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        let _ = ReleaseDC(None, hdc);
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        if lines == 0 {
            return Err(anyhow!("GetDIBits failed"));
        }
        let rgba = bgra_premul_to_rgba(&buf);
        let sq = fit_rgba_square(&rgba, bw as usize, bh as usize, ICON_SIZE);
        encode_png_rgba(&sq, ICON_SIZE as u32, ICON_SIZE as u32)
    }
}

/// 96x96 PNG for an app. Prefers the target exe's icon (no shortcut arrow overlay).
pub fn app_icon_png(app: &AppInfo) -> Result<Vec<u8>> {
    let app = app.clone();
    run_sta(move || {
        if let Some(aumid) = &app.aumid {
            return icon_for_path(Path::new(&format!("shell:AppsFolder\\{aumid}")));
        }
        if !app.target.is_empty() && Path::new(&app.target).exists() {
            if let Ok(p) = icon_for_path(Path::new(&app.target)) {
                return Ok(p);
            }
        }
        icon_for_path(&app.lnk)
    })
}

// ---------------------------------------------------------------------------------------
// Launching

pub struct Launched {
    pub pid: Option<u32>,
}

/// Launch a packaged app by AUMID; gives the PID of the app process. Call on a COM thread.
pub fn activate_packaged(aumid: &str) -> Result<Launched> {
    unsafe {
        let mgr: IApplicationActivationManager = CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)?;
        let w = wide(aumid);
        match mgr.ActivateApplication(PCWSTR(w.as_ptr()), PCWSTR::null(), AO_NONE) {
            Ok(pid) => Ok(Launched { pid: if pid == 0 { None } else { Some(pid) } }),
            Err(e) => {
                tracing::debug!("ActivateApplication({aumid}) failed: {e}; trying shell:AppsFolder");
                shell_launch(&format!("shell:AppsFolder\\{aumid}"), "")
            }
        }
    }
}

/// Launch a shortcut or executable via ShellExecuteExW. Call on a COM-initialised thread.
pub fn shell_launch(file: &str, args: &str) -> Result<Launched> {
    unsafe {
        let wfile = wide(file);
        let wargs = wide(args);
        let mut sei = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpFile: PCWSTR(wfile.as_ptr()),
            lpParameters: if args.is_empty() { PCWSTR::null() } else { PCWSTR(wargs.as_ptr()) },
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        ShellExecuteExW(&mut sei).map_err(|e| anyhow!("ShellExecuteEx failed: {e}"))?;
        let h: HANDLE = sei.hProcess;
        let pid = if h.is_invalid() || h.0.is_null() {
            None
        } else {
            let p = GetProcessId(h);
            let _ = CloseHandle(h);
            if p == 0 { None } else { Some(p) }
        };
        Ok(Launched { pid })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str) -> AppInfo {
        AppInfo {
            id: id.into(),
            name: name.into(),
            lnk: PathBuf::from(format!("{name}.lnk")),
            target: String::new(),
            exe_name: None,
            aumid: None,
        }
    }

    #[test]
    fn lnk_wins_over_appsfolder_entry_of_same_name() {
        let mut packaged = app("Notepad", "p");
        packaged.aumid = Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App".into());
        let out = sort_dedupe(vec![packaged, app("notepad", "z-lnk")]);
        assert_eq!(out.len(), 1);
        assert!(out[0].aumid.is_none(), "the .lnk entry must be kept");
        let mut calc = app("Calculator", "c");
        calc.aumid = Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into());
        assert_eq!(sort_dedupe(vec![calc, app("Terminal", "t")]).len(), 2);
    }

    #[test]
    fn aumid_detection() {
        assert!(is_aumid("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"));
        assert!(is_aumid("Microsoft.Windows.Explorer"));
        assert!(!is_aumid(r"C:\Windows\notepad.exe"));
        assert!(!is_aumid(r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\notepad.exe"));
        assert!(!is_aumid(""));
        assert_ne!(packaged_id("a"), stable_id("a"));
    }

    /// Lists shell:AppsFolder on this machine (never launches anything).
    #[test]
    fn apps_folder_lists_packaged_apps() {
        let v = crate::winutil::run_sta(|| scan_apps_folder().unwrap());
        println!("AppsFolder: {} entries", v.len());
        for a in &v {
            println!("  {:<40} {}", a.name, a.aumid.as_deref().unwrap_or(""));
        }
        assert!(!v.is_empty(), "no packaged apps found");
        assert!(v.iter().all(|a| a.aumid.is_some() && a.lnk.as_os_str().is_empty()));
        let has = |n: &str| v.iter().any(|a| a.name.to_lowercase().contains(n));
        assert!(has("calculator") || has("terminal") || has("notepad"), "no Calculator/Terminal/Notepad found");
    }

    #[test]
    fn merged_scan_has_packaged_and_lnk_entries() {
        let v = scan_apps();
        println!("merged: {} apps", v.len());
        assert!(v.iter().any(|a| a.aumid.is_some()), "packaged apps missing from merged list");
        assert!(v.iter().any(|a| a.aumid.is_none()), "Start Menu shortcuts missing from merged list");
        let mut names: Vec<_> = v.iter().map(|a| a.name.to_lowercase()).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(n, names.len(), "duplicate names in merged list");
    }

    #[test]
    fn stable_id_is_stable_and_case_insensitive() {
        let a = stable_id(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Notepad.lnk");
        let b = stable_id(r"c:\programdata\microsoft\windows\start menu\programs\notepad.lnk");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert_ne!(a, stable_id(r"C:\other.lnk"));
        // regression guard: value must not change between releases (ids are stored by phones)
        assert_eq!(stable_id("a"), "af63dc4c8601ec8c");
    }

    #[test]
    fn skips_uninstallers_and_docs() {
        assert!(should_skip("Uninstall Foo", r"C:\x\foo.exe"));
        assert!(should_skip("Foo Readme", ""));
        assert!(should_skip("Foo Help", r"C:\x\foo.exe"));
        assert!(should_skip("Foo", r"C:\x\unins000.exe"));
        assert!(should_skip("Foo", r"C:\x\notes.txt"));
        assert!(should_skip("Foo", r"C:\x\docs.chm"));
        assert!(!should_skip("Visual Studio Code", r"C:\Users\a\Code.exe"));
        assert!(!should_skip("Notepad", r"C:\Windows\notepad.exe"));
        assert!(!should_skip("Windows Terminal", ""));
    }

    #[test]
    fn sorts_and_dedupes() {
        let apps = vec![app("zed", "1"), app("Alpha", "2"), app("alpha", "3"), app("Beta", "4"), app("Beta", "4")];
        let out = sort_dedupe(apps);
        let names: Vec<_> = out.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Beta", "zed"]);
    }

    #[test]
    fn env_expansion() {
        unsafe { std::env::set_var("PENLINK_TEST_X", "abc") };
        assert_eq!(expand_env(r"%PENLINK_TEST_X%\a.exe"), r"abc\a.exe");
        assert_eq!(expand_env("plain"), "plain");
        assert_eq!(expand_env("%NOPE_NOT_SET_1%x"), "%NOPE_NOT_SET_1%x");
    }

    #[test]
    fn rgba_fit_pads_and_scales() {
        let src = vec![255u8; 2 * 2 * 4];
        let out = fit_rgba_square(&src, 2, 2, 4);
        assert_eq!(out.len(), 4 * 4 * 4);
        assert_eq!(&out[0..4], &[255, 255, 255, 255]);
        // non-square: 4x2 into 4x4 -> padded top and bottom
        let src = vec![9u8; 4 * 2 * 4];
        let out = fit_rgba_square(&src, 4, 2, 4);
        assert_eq!(out[3], 0); // top row transparent
        assert_eq!(out[(4 + 1) * 4 + 3], 9); // row 1 has content
    }

    #[test]
    fn premultiplied_conversion() {
        // half-alpha premultiplied red (B=0,G=0,R=64,A=128) -> straight R~127
        let out = bgra_premul_to_rgba(&[0, 0, 64, 128]);
        assert_eq!(out[3], 128);
        assert!((out[0] as i32 - 127).abs() <= 1);
        // no alpha anywhere -> opaque
        let out = bgra_premul_to_rgba(&[1, 2, 3, 0]);
        assert_eq!(out, vec![3, 2, 1, 255]);
    }

    #[test]
    fn png_encodes() {
        let rgba = vec![0u8; 96 * 96 * 4];
        let png = encode_png_rgba(&rgba, 96, 96).unwrap();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    }
}
