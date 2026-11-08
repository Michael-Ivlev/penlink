//! SudoVDA virtual display driver (https://github.com/SudoMaker/SudoVDA): open it, keep its
//! watchdog fed, add and remove monitors, and the Win32 side of a monitor's life (find it,
//! position it, set its DPI). The decisions live in `sizing` and `vdmanager`; this file is
//! the part that talks to Windows.
//!
//! Safety rules: only monitors this driver created are ever touched. The position, mode and
//! DPI calls refuse the primary monitor, and always name the one device they change.

use crate::geom::Rect;
use crate::sizing::{pick_dpi_step, pick_render_adapter, AdapterInfo, DpiRange};
use crate::vdledger::{self, Guid, Ledger};
use crate::vdmanager::{AddedDisplay, Backend, VdManager};
use crate::winutil::{self, from_wide, wide};
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{BOOL, GUID, HRESULT, PCWSTR};
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, LPARAM, LUID, POINTL, RECT, WAIT_TIMEOUT};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, CDS_NORESET, CDS_TEST,
    CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS,
    HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

// ---------------------------------------------------------------------------------------
// Protocol with the driver (Common/Include/sudovda-ioctl.h)

/// {e5bcc234-1e0c-418a-a0d4-ef8b7501414d}
pub const INTERFACE_GUID: GUID = GUID::from_u128(0xe5bcc234_1e0c_418a_a0d4_ef8b7501414d);

/// CTL_CODE(FILE_DEVICE_UNKNOWN, function, METHOD_BUFFERED, FILE_ANY_ACCESS)
pub const fn ctl_code(function: u32) -> u32 {
    (0x22 << 16) | (function << 2)
}
pub const IOCTL_ADD_VIRTUAL_DISPLAY: u32 = ctl_code(0x800);
pub const IOCTL_REMOVE_VIRTUAL_DISPLAY: u32 = ctl_code(0x801);
pub const IOCTL_SET_RENDER_ADAPTER: u32 = ctl_code(0x802);
pub const IOCTL_GET_WATCHDOG: u32 = ctl_code(0x803);
pub const IOCTL_DRIVER_PING: u32 = ctl_code(0x888);
pub const IOCTL_GET_PROTOCOL_VERSION: u32 = ctl_code(0x8FF);

/// Protocol version this code was written against (`VDAProtocolVersion`).
pub const PROTOCOL: (u8, u8, u8) = (0, 2, 1);

#[repr(C)]
#[derive(Clone, Copy)]
struct AddParams {
    width: u32,
    height: u32,
    refresh_rate: u32,
    monitor_guid: GUID,
    device_name: [u8; 14],
    serial_number: [u8; 14],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AddOut {
    adapter_luid: LUID,
    target_id: u32,
}

// Version (4 bytes: major, minor, incremental, test build) and watchdog (two u32: timeout in
// seconds, countdown) answers are read from plain byte arrays below.

/// `isProtocolCompatible`: same major, and the driver is not older (minor) than this code.
pub fn protocol_compatible(driver: (u8, u8, u8)) -> bool {
    driver.0 == PROTOCOL.0 && PROTOCOL.1 <= driver.1
}

fn c_name(s: &str) -> [u8; 14] {
    let mut out = [0u8; 14];
    for (i, b) in s.bytes().take(13).enumerate() {
        out[i] = b;
    }
    out
}

fn to_win_guid(g: &Guid) -> GUID {
    GUID { data1: g.d1, data2: g.d2, data3: g.d3, data4: g.d4 }
}

// ---------------------------------------------------------------------------------------
// Opening the device

/// Path of the first SudoVDA device interface, `None` when the driver is not installed.
pub fn find_device_path() -> Option<Vec<u16>> {
    unsafe {
        let set = SetupDiGetClassDevsW(Some(&INTERFACE_GUID), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE).ok()?;
        let mut result = None;
        let mut data = SP_DEVICE_INTERFACE_DATA { cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
        let mut index = 0;
        while SetupDiEnumDeviceInterfaces(set, None, &INTERFACE_GUID, index, &mut data).is_ok() {
            index += 1;
            let mut needed = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut needed), None);
            if needed == 0 {
                continue;
            }
            // u64 buffer: the detail struct wants 4/8 byte alignment
            let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &data, Some(detail), needed, None, None).is_ok() {
                let path_ptr = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
                let max = (needed as usize - 4) / 2;
                let slice = std::slice::from_raw_parts(path_ptr, max);
                let n = slice.iter().position(|&c| c == 0).unwrap_or(max);
                let mut p = slice[..n].to_vec();
                p.push(0);
                result = Some(p);
                break;
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        result
    }
}

pub fn is_installed() -> bool {
    find_device_path().is_some()
}

/// One DeviceIoControl, waiting for completion (the handle is overlapped). Gives up after 10 s.
fn ioctl(handle: isize, code: u32, input: &[u8], output: &mut [u8]) -> Result<u32> {
    unsafe {
        let h = HANDLE(handle as *mut c_void);
        let ev = CreateEventW(None, true, false, PCWSTR::null()).context("CreateEventW")?;
        let mut ov = OVERLAPPED { hEvent: ev, ..Default::default() };
        let mut returned = 0u32;
        let r = DeviceIoControl(
            h,
            code,
            (!input.is_empty()).then_some(input.as_ptr() as *const c_void),
            input.len() as u32,
            (!output.is_empty()).then_some(output.as_mut_ptr() as *mut c_void),
            output.len() as u32,
            Some(&mut returned),
            Some(&mut ov),
        );
        let res = match r {
            Ok(()) => Ok(returned),
            Err(e) if e.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                if WaitForSingleObject(ev, 10_000) == WAIT_TIMEOUT {
                    let _ = CancelIoEx(h, Some(&ov));
                }
                GetOverlappedResult(h, &ov, &mut returned, true).map(|_| returned).map_err(|e| anyhow!("ioctl {code:#x}: {e}"))
            }
            Err(e) => Err(anyhow!("ioctl {code:#x}: {e}")),
        };
        let _ = CloseHandle(ev);
        res
    }
}

/// An open driver handle plus the thread that pings its watchdog.
pub struct Driver {
    handle: isize,
    stop: Arc<AtomicBool>,
    pub healthy: Arc<AtomicBool>,
    ping: Option<std::thread::JoinHandle<()>>,
    pub watchdog_secs: u32,
    pub protocol: (u8, u8, u8),
}

impl Driver {
    /// Open the device, check the protocol version, start the ping thread, pin the render
    /// adapter. `Err` when the driver is missing or too old.
    pub fn open() -> Result<Driver> {
        let path = find_device_path().ok_or_else(|| anyhow!("SudoVDA is not installed"))?;
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .context("cannot open the SudoVDA device")?;
        let handle = handle.0 as isize;

        let mut ver = [0u8; 4];
        ioctl(handle, IOCTL_GET_PROTOCOL_VERSION, &[], &mut ver)?;
        let protocol = (ver[0], ver[1], ver[2]);
        if !protocol_compatible(protocol) {
            unsafe { let _ = CloseHandle(HANDLE(handle as *mut c_void)); }
            bail!("SudoVDA protocol {protocol:?} is not compatible with {PROTOCOL:?}");
        }

        let mut wd = [0u8; 8];
        let mut watchdog_secs = 0;
        if ioctl(handle, IOCTL_GET_WATCHDOG, &[], &mut wd).is_ok() {
            watchdog_secs = u32::from_le_bytes([wd[0], wd[1], wd[2], wd[3]]);
        } else {
            tracing::warn!("SudoVDA watchdog query failed");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let healthy = Arc::new(AtomicBool::new(true));
        let ping = if watchdog_secs > 0 {
            // Ping three times per watchdog period; the first one right away.
            let interval = Duration::from_millis((watchdog_secs as u64 * 1000 / 3).max(200));
            let (stop, healthy) = (stop.clone(), healthy.clone());
            std::thread::Builder::new()
                .name("penlink-vd-ping".into())
                .spawn(move || {
                    let mut fails = 0;
                    while !stop.load(Ordering::SeqCst) {
                        match ioctl(handle, IOCTL_DRIVER_PING, &[], &mut []) {
                            Ok(_) => {
                                fails = 0;
                                healthy.store(true, Ordering::SeqCst);
                            }
                            Err(e) => {
                                fails += 1;
                                tracing::warn!("SudoVDA ping failed ({fails}): {e}");
                                if fails > 3 {
                                    healthy.store(false, Ordering::SeqCst);
                                }
                            }
                        }
                        // sleep in small slices so shutdown is quick
                        let until = Instant::now() + interval;
                        while Instant::now() < until && !stop.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                })
                .ok()
        } else {
            None
        };
        let d = Driver { handle, stop, healthy, ping, watchdog_secs, protocol };
        d.pin_render_adapter();
        Ok(d)
    }

    fn pin_render_adapter(&self) {
        let list = list_adapters();
        let infos: Vec<AdapterInfo> = list.iter().map(|(i, _)| i.clone()).collect();
        let pref = std::env::var("PENLINK_RENDER_ADAPTER").ok();
        let Some(i) = pick_render_adapter(&infos, pref.as_deref()) else {
            tracing::warn!("no hardware GPU found to pin the virtual monitors to");
            return;
        };
        let luid = list[i].1;
        let mut buf = [0u8; 8];
        buf[..4].copy_from_slice(&luid.LowPart.to_le_bytes());
        buf[4..].copy_from_slice(&luid.HighPart.to_le_bytes());
        match ioctl(self.handle, IOCTL_SET_RENDER_ADAPTER, &buf, &mut []) {
            Ok(_) => tracing::info!("virtual monitors render on {:?}", list[i].0.description),
            Err(e) => tracing::warn!("could not pin the render adapter: {e}"),
        }
    }

    fn add(&self, guid: &Guid, width: u32, height: u32, refresh: u32) -> Result<AddOut> {
        let p = AddParams {
            width,
            height,
            refresh_rate: refresh,
            monitor_guid: to_win_guid(guid),
            device_name: c_name(vdledger::DEVICE_NAME),
            serial_number: c_name(&guid.serial()),
        };
        let bytes = unsafe { std::slice::from_raw_parts(&p as *const AddParams as *const u8, std::mem::size_of::<AddParams>()) };
        let mut out = AddOut::default();
        let out_bytes = unsafe { std::slice::from_raw_parts_mut(&mut out as *mut AddOut as *mut u8, std::mem::size_of::<AddOut>()) };
        ioctl(self.handle, IOCTL_ADD_VIRTUAL_DISPLAY, bytes, out_bytes).context("add virtual display")?;
        Ok(out)
    }

    fn remove(&self, guid: &Guid) -> Result<()> {
        let g = to_win_guid(guid);
        let bytes = unsafe { std::slice::from_raw_parts(&g as *const GUID as *const u8, std::mem::size_of::<GUID>()) };
        ioctl(self.handle, IOCTL_REMOVE_VIRTUAL_DISPLAY, bytes, &mut []).map(|_| ())
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.ping.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = CloseHandle(HANDLE(self.handle as *mut c_void));
        }
    }
}

// ---------------------------------------------------------------------------------------
// GPUs

/// Hardware and software adapters with their LUIDs.
pub fn list_adapters() -> Vec<(AdapterInfo, LUID)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            if let Ok(d) = a.GetDesc1() {
                out.push((
                    AdapterInfo {
                        description: from_wide(&d.Description),
                        vendor_id: d.VendorId,
                        dedicated_vram: d.DedicatedVideoMemory as u64,
                        software: d.Flags & 2 != 0,
                    },
                    d.AdapterLuid,
                ));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Monitors

#[derive(Debug, Clone)]
pub struct MonInfo {
    pub hmonitor: isize,
    pub name: String,
    pub rect: Rect,
    pub primary: bool,
}

pub fn enum_monitors() -> Vec<MonInfo> {
    unsafe extern "system" fn cb(hmon: HMONITOR, _hdc: HDC, _rc: *mut RECT, lp: LPARAM) -> BOOL {
        let v = unsafe { &mut *(lp.0 as *mut Vec<MonInfo>) };
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(hmon, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool() {
            let r = mi.monitorInfo.rcMonitor;
            v.push(MonInfo {
                hmonitor: hmon.0 as isize,
                name: from_wide(&mi.szDevice),
                rect: Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top),
                primary: mi.monitorInfo.dwFlags & 1 != 0,
            });
        }
        true.into()
    }
    let mut v: Vec<MonInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut Vec<MonInfo> as isize));
    }
    v
}

/// The part of a monitor that apps can use (`rcWork`): the monitor minus the taskbar Windows
/// shows on every display. Penlink streams only this area, so the phone never shows the taskbar.
pub fn work_rect(hmonitor: isize) -> Option<Rect> {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let ok = unsafe { GetMonitorInfoW(HMONITOR(hmonitor as *mut std::ffi::c_void), &mut mi) }.as_bool();
    let r = mi.rcWork;
    (ok && r.right > r.left && r.bottom > r.top).then(|| Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// Whether Windows shows its taskbar on every display (Settings > Taskbar > "Show my taskbar on
/// all displays", on unless the user turned it off).
fn taskbar_on_all_displays() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut v: u32 = 0;
    let mut n = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"),
            w!("MMTaskbarEnabled"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut std::ffi::c_void),
            Some(&mut n),
        )
    };
    r.is_err() || v != 0
}

/// The work area of a freshly (re)created virtual monitor. Explorer adds the taskbar a moment
/// after the monitor appears (and shrinks the maximised window), so wait for it, up to
/// `max`. Without the "taskbar on all displays" setting there is nothing to wait for.
pub fn settled_work_rect(hmonitor: isize, mon: Rect, max: Duration) -> Rect {
    if taskbar_on_all_displays() {
        let deadline = std::time::Instant::now() + max;
        while std::time::Instant::now() < deadline {
            if work_rect(hmonitor).is_some_and(|w| w != mon) {
                std::thread::sleep(Duration::from_millis(250));
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    work_rect(hmonitor).unwrap_or(mon)
}

pub fn monitor_by_name(name: &str) -> Option<MonInfo> {
    enum_monitors().into_iter().find(|m| m.name.eq_ignore_ascii_case(name))
}

/// Text lines describing every monitor (name, rectangle, primary, refresh rate), in a stable
/// order. Used to prove that the real monitors did not change.
pub fn monitor_snapshot() -> Vec<String> {
    let mut v: Vec<String> = enum_monitors()
        .into_iter()
        .map(|m| {
            let hz = current_mode(&m.name).map(|d| d.dmDisplayFrequency).unwrap_or(0);
            format!("{} {}x{} at {},{} primary={} {}Hz", m.name, m.rect.w, m.rect.h, m.rect.x, m.rect.y, m.primary, hz)
        })
        .collect();
    v.sort();
    v
}

fn current_mode(name: &str) -> Option<DEVMODEW> {
    let w = wide(name);
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    unsafe { EnumDisplaySettingsW(PCWSTR(w.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm) }.as_bool().then_some(dm)
}

fn active_paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>> {
    for _ in 0..4 {
        let (mut np, mut nm) = (0u32, 0u32);
        let e = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) };
        if e.0 != 0 {
            bail!("GetDisplayConfigBufferSizes: {}", e.0);
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = (0..nm).map(|_| unsafe { std::mem::zeroed() }).collect();
        let e = unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) };
        match e.0 {
            0 => {
                paths.truncate(np as usize);
                return Ok(paths);
            }
            122 => continue, // ERROR_INSUFFICIENT_BUFFER: the topology changed meanwhile
            c => bail!("QueryDisplayConfig: {c}"),
        }
    }
    bail!("QueryDisplayConfig kept changing")
}

fn source_gdi_name(adapter: LUID, source_id: u32) -> Option<String> {
    let mut n = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    n.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    n.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    n.header.adapterId = adapter;
    n.header.id = source_id;
    (unsafe { DisplayConfigGetDeviceInfo(&mut n.header) } == 0).then(|| from_wide(&n.viewGdiDeviceName))
}

/// (adapter, source id) of the active path that feeds this GDI device.
fn find_source(gdi_name: &str) -> Option<(LUID, u32)> {
    for p in active_paths().ok()? {
        if source_gdi_name(p.sourceInfo.adapterId, p.sourceInfo.id).is_some_and(|n| n.eq_ignore_ascii_case(gdi_name)) {
            return Some((p.sourceInfo.adapterId, p.sourceInfo.id));
        }
    }
    None
}

fn poll<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    let mut wait = 20;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if start.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(wait));
        wait = (wait * 2).min(250);
    }
}

// ---------------------------------------------------------------------------------------
// DPI (undocumented DisplayConfig device-info types -3 and -4, see imniko/SetDPI)

#[repr(C)]
struct DpiGet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    min_rel: i32,
    cur_rel: i32,
    max_rel: i32,
}

#[repr(C)]
struct DpiSet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    scale_rel: i32,
}

const GET_DPI_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-3);
const SET_DPI_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-4);

fn get_dpi_range(adapter: LUID, source_id: u32) -> Result<DpiRange> {
    debug_assert_eq!(std::mem::size_of::<DpiGet>(), 0x20);
    let mut p = DpiGet {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: GET_DPI_SCALE, size: std::mem::size_of::<DpiGet>() as u32, adapterId: adapter, id: source_id },
        min_rel: 0,
        cur_rel: 0,
        max_rel: 0,
    };
    let e = unsafe { DisplayConfigGetDeviceInfo(&mut p.header) };
    if e != 0 {
        bail!("DisplayConfigGetDeviceInfo(DPI) failed: {e}");
    }
    Ok(DpiRange { min_rel: p.min_rel, cur_rel: p.cur_rel, max_rel: p.max_rel })
}

fn set_dpi_rel(adapter: LUID, source_id: u32, rel: i32) -> Result<()> {
    debug_assert_eq!(std::mem::size_of::<DpiSet>(), 0x18);
    let p = DpiSet {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: SET_DPI_SCALE, size: std::mem::size_of::<DpiSet>() as u32, adapterId: adapter, id: source_id },
        scale_rel: rel,
    };
    let e = unsafe { DisplayConfigSetDeviceInfo(&p.header) };
    if e != 0 {
        bail!("DisplayConfigSetDeviceInfo(DPI) failed: {e}");
    }
    Ok(())
}

/// Current DPI percentage of a monitor, if Windows will tell.
pub fn dpi_percent(gdi_name: &str) -> Option<u32> {
    let (a, s) = find_source(gdi_name)?;
    get_dpi_range(a, s).ok()?.current_percent()
}

// ---------------------------------------------------------------------------------------
// The backend the manager drives

pub struct WinBackend {
    driver: Driver,
}

impl WinBackend {
    fn refuse_primary(&self, gdi_name: &str) -> Result<MonInfo> {
        let m = monitor_by_name(gdi_name).ok_or_else(|| anyhow!("monitor {gdi_name} not found"))?;
        if m.primary {
            bail!("refusing to change the primary monitor {gdi_name}");
        }
        Ok(m)
    }
}

impl Backend for WinBackend {
    fn add(&self, guid: &Guid, width: u32, height: u32, refresh: u32) -> Result<AddedDisplay> {
        let out = self.driver.add(guid, width, height, refresh)?;
        let found = poll(Duration::from_secs(8), || {
            let paths = active_paths().ok()?;
            let p = paths.iter().find(|p| p.targetInfo.id == out.target_id && p.targetInfo.adapterId.LowPart == out.adapter_luid.LowPart && p.targetInfo.adapterId.HighPart == out.adapter_luid.HighPart)?;
            let name = source_gdi_name(p.sourceInfo.adapterId, p.sourceInfo.id)?;
            let m = monitor_by_name(&name)?;
            Some(AddedDisplay { gdi_name: name, hmonitor: m.hmonitor, rect: m.rect })
        });
        match found {
            Some(d) => Ok(d),
            None => {
                let _ = self.driver.remove(guid);
                bail!("the virtual monitor did not become active")
            }
        }
    }

    fn remove(&self, guid: &Guid, gdi_name: Option<&str>) -> Result<()> {
        self.driver.remove(guid)?;
        if let Some(n) = gdi_name {
            // Windows needs a moment to drop it from the desktop.
            let _ = poll(Duration::from_secs(4), || monitor_by_name(n).is_none().then_some(()));
        }
        Ok(())
    }

    fn other_monitor_rects(&self, except_gdi: &str) -> Vec<Rect> {
        enum_monitors().into_iter().filter(|m| !m.name.eq_ignore_ascii_case(except_gdi)).map(|m| m.rect).collect()
    }

    fn position(&self, gdi_name: &str, x: i32, y: i32) -> Result<()> {
        let m = self.refuse_primary(gdi_name)?;
        if m.rect.x == x && m.rect.y == y {
            return Ok(());
        }
        let name = wide(gdi_name);
        let mut dm = current_mode(gdi_name).ok_or_else(|| anyhow!("no current mode for {gdi_name}"))?;
        dm.dmFields = DM_POSITION;
        dm.Anonymous1.Anonymous2.dmPosition = POINTL { x, y };
        let r = unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_UPDATEREGISTRY | CDS_NORESET, None) };
        if r != DISP_CHANGE_SUCCESSFUL {
            bail!("ChangeDisplaySettingsEx(position) returned {}", r.0);
        }
        // Apply the pending registry change (only this device has one).
        let r = unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };
        if r != DISP_CHANGE_SUCCESSFUL {
            bail!("applying the monitor position returned {}", r.0);
        }
        Ok(())
    }

    fn set_dpi(&self, gdi_name: &str, wanted: u32) -> Result<u32> {
        self.refuse_primary(gdi_name)?;
        let (adapter, source) = poll(Duration::from_secs(3), || find_source(gdi_name)).ok_or_else(|| anyhow!("no display source for {gdi_name}"))?;
        let range = get_dpi_range(adapter, source)?;
        let (rel, pct) = pick_dpi_step(wanted, &range).ok_or_else(|| anyhow!("unusable DPI range {range:?}"))?;
        tracing::info!("{gdi_name}: DPI wanted {wanted}%, allowed {:?}, using {pct}%", range.allowed());
        if range.current_percent() != Some(pct) {
            set_dpi_rel(adapter, source, rel)?;
        }
        // Report what Windows really applied.
        Ok(get_dpi_range(adapter, source).ok().and_then(|r| r.current_percent()).unwrap_or(pct))
    }

    fn refresh(&self, gdi_name: &str) -> Result<(isize, Rect)> {
        monitor_by_name(gdi_name).map(|m| (m.hmonitor, m.rect)).ok_or_else(|| anyhow!("monitor {gdi_name} is gone"))
    }

    fn try_set_mode(&self, gdi_name: &str, width: u32, height: u32) -> bool {
        if self.refuse_primary(gdi_name).is_err() {
            return false;
        }
        let Some(mut dm) = current_mode(gdi_name) else { return false };
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT;
        dm.dmPelsWidth = width;
        dm.dmPelsHeight = height;
        let name = wide(gdi_name);
        let test = unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_TEST, None) };
        if test != DISP_CHANGE_SUCCESSFUL {
            return false;
        }
        let r = unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_UPDATEREGISTRY, None) };
        r == DISP_CHANGE_SUCCESSFUL
    }

    fn return_window(&self, hwnd: isize, from: Rect) {
        winutil::return_window_to_primary(hwnd, from);
    }

    fn window_alive(&self, hwnd: isize) -> bool {
        winutil::is_window(hwnd)
    }
}

/// Open the driver and build the manager, removing monitors an earlier run left behind.
/// `None` when the driver is not installed or unusable (the host then uses window capture).
pub fn start_manager() -> Option<Arc<VdManager>> {
    if std::env::var_os("PENLINK_NO_VDISPLAY").is_some() {
        tracing::info!("virtual monitors disabled (PENLINK_NO_VDISPLAY)");
        return None;
    }
    let driver = match Driver::open() {
        Ok(d) => d,
        Err(e) => {
            tracing::info!("virtual monitors unavailable, using window capture: {e:#}");
            return None;
        }
    };
    tracing::info!(
        "SudoVDA protocol {}.{}.{}, watchdog {} s",
        driver.protocol.0,
        driver.protocol.1,
        driver.protocol.2,
        driver.watchdog_secs
    );
    let mgr = VdManager::new(Arc::new(WinBackend { driver }), Ledger::load(&vdledger::default_path()));
    mgr.cleanup_leftovers();
    Some(mgr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_codes_match_ctl_code() {
        // CTL_CODE(FILE_DEVICE_UNKNOWN=0x22, f, METHOD_BUFFERED, FILE_ANY_ACCESS)
        assert_eq!(IOCTL_ADD_VIRTUAL_DISPLAY, 0x0022_2000);
        assert_eq!(IOCTL_REMOVE_VIRTUAL_DISPLAY, 0x0022_2004);
        assert_eq!(IOCTL_SET_RENDER_ADAPTER, 0x0022_2008);
        assert_eq!(IOCTL_GET_WATCHDOG, 0x0022_200C);
        assert_eq!(IOCTL_DRIVER_PING, 0x0022_2220);
        assert_eq!(IOCTL_GET_PROTOCOL_VERSION, 0x0022_23FC);
    }

    #[test]
    fn struct_layouts_match_the_driver_header() {
        assert_eq!(std::mem::size_of::<AddParams>(), 56);
        assert_eq!(std::mem::size_of::<AddOut>(), 12);
        assert_eq!(std::mem::size_of::<DpiGet>(), 0x20);
        assert_eq!(std::mem::size_of::<DpiSet>(), 0x18);
        assert_eq!(std::mem::size_of::<DISPLAYCONFIG_DEVICE_INFO_HEADER>(), 20);
    }

    #[test]
    fn interface_guid_is_the_drivers() {
        assert_eq!(INTERFACE_GUID.to_u128(), 0xe5bcc234_1e0c_418a_a0d4_ef8b7501414d);
    }

    #[test]
    fn protocol_check_like_the_reference() {
        assert!(protocol_compatible((0, 2, 1)));
        assert!(protocol_compatible((0, 3, 0)));
        assert!(!protocol_compatible((0, 1, 9)));
        assert!(!protocol_compatible((1, 2, 1)));
    }

    #[test]
    fn names_are_truncated_to_13_chars_and_terminated() {
        let n = c_name("0123456789ABCDEFGH");
        assert_eq!(&n[..13], b"0123456789ABC");
        assert_eq!(n[13], 0);
    }
}
