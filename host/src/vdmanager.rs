//! Lifecycle of the virtual monitors: create (placed right of the real ones, DPI set), change
//! size, keep for a grace period after the phone dropped, remove, and clean up what a crashed
//! host left behind. All driver and Win32 work goes through [`Backend`], so the logic is tested
//! with a fake.

use crate::geom::Rect;
use crate::sizing::place_right_of;
use crate::vdledger::{Guid, Ledger};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a monitor is kept after the phone's WebSocket dropped without a goodbye, so a short
/// network blip can attach to it again. After that the monitor is removed and the app window goes
/// back to the PC (a phone that comes back later gets a new monitor).
pub const GRACE: Duration = Duration::from_secs(5);
pub const REFRESH_RATE: u32 = 60;

/// A virtual monitor as it is now.
#[derive(Debug, Clone, PartialEq)]
pub struct VMonitor {
    pub guid: Guid,
    /// GDI device name, e.g. `\\.\DISPLAY7`.
    pub gdi_name: String,
    pub hmonitor: isize,
    /// Position and size in virtual-desktop pixels.
    pub rect: Rect,
    /// DPI percent that was asked for, and the step Windows really applied.
    pub dpi_wanted: u32,
    pub dpi_actual: u32,
}

/// What the driver reported for a monitor it just added.
#[derive(Debug, Clone, PartialEq)]
pub struct AddedDisplay {
    pub gdi_name: String,
    pub hmonitor: isize,
    pub rect: Rect,
}

pub trait Backend: Send + Sync {
    /// Ask the driver for a monitor and wait until Windows lists it as active.
    fn add(&self, guid: &Guid, width: u32, height: u32, refresh: u32) -> Result<AddedDisplay>;
    /// Remove it and wait until Windows no longer lists it.
    fn remove(&self, guid: &Guid, gdi_name: Option<&str>) -> Result<()>;
    /// Rectangles of all monitors except the one with this GDI name.
    fn other_monitor_rects(&self, except_gdi: &str) -> Vec<Rect>;
    /// Move that monitor (only that one, never the primary) to a new top-left.
    fn position(&self, gdi_name: &str, x: i32, y: i32) -> Result<()>;
    /// Set the nearest allowed DPI step; returns the percentage that was applied.
    fn set_dpi(&self, gdi_name: &str, wanted: u32) -> Result<u32>;
    /// Current HMONITOR and rectangle of a monitor, `Err` when it is not there.
    fn refresh(&self, gdi_name: &str) -> Result<(isize, Rect)>;
    /// Try to switch the existing monitor to a new resolution (tested first, applied only if
    /// Windows accepts it). `false`: not possible, the caller removes and re-adds it.
    fn try_set_mode(&self, gdi_name: &str, width: u32, height: u32) -> bool;
    /// Put an app window back on the primary monitor, if it is on `from` (the monitor that is
    /// about to go).
    fn return_window(&self, hwnd: isize, from: Rect);
    fn window_alive(&self, hwnd: isize) -> bool;
}

struct Lease {
    mon: VMonitor,
    app_hwnd: isize,
    app_id: String,
    /// Set while the phone is away: the monitor is removed at this time.
    orphan_deadline: Option<Instant>,
}

struct Inner {
    ledger: Ledger,
    leases: HashMap<Guid, Lease>,
    /// Monitors whose removal failed; [`VdManager::reap`] tries again.
    stuck: Vec<(Guid, String, u32)>,
}

pub struct VdManager {
    backend: Arc<dyn Backend>,
    inner: Mutex<Inner>,
}

impl VdManager {
    pub fn new(backend: Arc<dyn Backend>, ledger: Ledger) -> Arc<VdManager> {
        Arc::new(VdManager { backend, inner: Mutex::new(Inner { ledger, leases: HashMap::new(), stuck: Vec::new() }) })
    }

    /// Remove every monitor an earlier run of the host left behind (the ledger lists them).
    /// Returns how many removals were attempted.
    pub fn cleanup_leftovers(&self) -> usize {
        let mut g = self.inner.lock().unwrap();
        let old = g.ledger.all();
        for guid in &old {
            match self.backend.remove(guid, None) {
                Ok(()) => tracing::info!("removed leftover virtual monitor {}", guid.format()),
                Err(e) => tracing::debug!("leftover virtual monitor {} not removed (probably gone): {e}", guid.format()),
            }
            g.ledger.remove(guid);
        }
        old.len()
    }

    /// Create a monitor of `width` x `height` with the DPI step nearest to `dpi_wanted`.
    pub fn create(&self, width: u32, height: u32, dpi_wanted: u32) -> Result<VMonitor> {
        let mut g = self.inner.lock().unwrap();
        let guid = Guid::penlink(crate::store::random_bytes::<8>());
        // Written before the driver is asked: a crash in between still leaves a trace.
        g.ledger.add(guid);
        match self.setup(&guid, width, height, dpi_wanted) {
            Ok(mon) => {
                g.leases.insert(guid, Lease { mon: mon.clone(), app_hwnd: 0, app_id: String::new(), orphan_deadline: None });
                Ok(mon)
            }
            Err((e, gdi)) => {
                match gdi {
                    // The driver never created it: nothing to remove.
                    None => g.ledger.remove(&guid),
                    // Added but unusable: take it out again; keep the trace if that fails.
                    Some(n) => {
                        if self.backend.remove(&guid, Some(&n)).is_ok() {
                            g.ledger.remove(&guid);
                        }
                    }
                }
                Err(e)
            }
        }
    }

    /// Add `guid` to the driver, place it, set the DPI. On failure the error carries the GDI
    /// name if the monitor was already added (so the caller removes it again).
    fn setup(&self, guid: &Guid, width: u32, height: u32, dpi_wanted: u32) -> std::result::Result<VMonitor, (anyhow::Error, Option<String>)> {
        let added = self.backend.add(guid, width, height, REFRESH_RATE).map_err(|e| (e, None))?;
        let gdi = added.gdi_name.clone();
        // Windows remembers the mode a monitor identity had last (a re-added GUID comes back at
        // its old size, e.g. after the phone rotated), whatever the driver was asked for.
        if added.rect.w != width as i32 || added.rect.h != height as i32 {
            tracing::info!("{gdi} came back as {}x{}, asking for {width}x{height}", added.rect.w, added.rect.h);
            if !self.backend.try_set_mode(&gdi, width, height) {
                return Err((anyhow!("Windows kept the virtual monitor at {}x{} instead of {width}x{height}", added.rect.w, added.rect.h), Some(gdi)));
            }
        }
        let (x, y) = place_right_of(&self.backend.other_monitor_rects(&gdi));
        if let Err(e) = self.backend.position(&gdi, x, y) {
            // Usable where Windows put it; the phone only needs the rectangle.
            tracing::warn!("could not position virtual monitor {gdi}: {e}");
        }
        let dpi_actual = match self.backend.set_dpi(&gdi, dpi_wanted) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("could not set DPI of {gdi}: {e}");
                100
            }
        };
        let (mut hmonitor, mut rect) = self.backend.refresh(&gdi).map_err(|e| (e, Some(gdi.clone())))?;
        // A mode switch needs a moment to reach the desktop: wait until the rectangle is the asked size.
        for _ in 0..20 {
            if rect.w == width as i32 && rect.h == height as i32 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            if let Ok((h, r)) = self.backend.refresh(&gdi) {
                hmonitor = h;
                rect = r;
            }
        }
        Ok(VMonitor { guid: *guid, gdi_name: gdi, hmonitor, rect, dpi_wanted, dpi_actual })
    }

    /// New size (and DPI) for an existing monitor: switch its mode if Windows allows it,
    /// otherwise remove it and add it again under the same GUID.
    pub fn reconfigure(&self, mon: &VMonitor, width: u32, height: u32, dpi_wanted: u32) -> Result<VMonitor> {
        let mut g = self.inner.lock().unwrap();
        let lease = g.leases.get(&mon.guid).ok_or_else(|| anyhow!("unknown virtual monitor"))?;
        let (app_hwnd, app_id, deadline) = (lease.app_hwnd, lease.app_id.clone(), lease.orphan_deadline);
        let new_mon = if self.backend.try_set_mode(&mon.gdi_name, width, height) {
            tracing::info!("virtual monitor {} switched to {width}x{height}", mon.gdi_name);
            let mut m = mon.clone();
            if let Ok(a) = self.backend.set_dpi(&m.gdi_name, dpi_wanted) {
                m.dpi_actual = a;
            }
            m.dpi_wanted = dpi_wanted;
            let (hmonitor, rect) = self.backend.refresh(&m.gdi_name)?;
            m.hmonitor = hmonitor;
            m.rect = rect;
            m
        } else {
            tracing::info!("virtual monitor {} re-created at {width}x{height}", mon.gdi_name);
            self.backend.remove(&mon.guid, Some(&mon.gdi_name))?;
            match self.setup(&mon.guid, width, height, dpi_wanted) {
                Ok(m) => m,
                Err((e, gdi)) => {
                    // Not usable: drop it completely.
                    let _ = self.backend.remove(&mon.guid, gdi.as_deref());
                    g.ledger.remove(&mon.guid);
                    g.leases.remove(&mon.guid);
                    return Err(e);
                }
            }
        };
        g.leases.insert(mon.guid, Lease { mon: new_mon.clone(), app_hwnd, app_id, orphan_deadline: deadline });
        Ok(new_mon)
    }

    /// Change only the DPI step of an existing monitor.
    pub fn set_dpi(&self, mon: &VMonitor, dpi_wanted: u32) -> Result<VMonitor> {
        let mut g = self.inner.lock().unwrap();
        let lease = g.leases.get_mut(&mon.guid).ok_or_else(|| anyhow!("unknown virtual monitor"))?;
        let actual = self.backend.set_dpi(&mon.gdi_name, dpi_wanted)?;
        let (hmonitor, rect) = self.backend.refresh(&mon.gdi_name)?;
        lease.mon.dpi_wanted = dpi_wanted;
        lease.mon.dpi_actual = actual;
        lease.mon.hmonitor = hmonitor;
        lease.mon.rect = rect;
        Ok(lease.mon.clone())
    }

    /// Record which app window lives on the monitor (so it can be adopted again and
    /// returned to the PC when the monitor goes).
    pub fn bind(&self, guid: &Guid, app_hwnd: isize, app_id: &str) {
        if let Some(l) = self.inner.lock().unwrap().leases.get_mut(guid) {
            l.app_hwnd = app_hwnd;
            l.app_id = app_id.to_string();
            l.orphan_deadline = None;
        }
    }

    /// Current state of a monitor.
    pub fn get(&self, guid: &Guid) -> Option<VMonitor> {
        self.inner.lock().unwrap().leases.get(guid).map(|l| l.mon.clone())
    }

    /// Remove the monitor now; the app window goes back to the primary monitor first.
    pub fn release(&self, guid: &Guid) {
        let mut g = self.inner.lock().unwrap();
        self.release_locked(&mut g, guid);
    }

    fn return_app_window(&self, lease: &Lease) {
        if lease.app_hwnd != 0 && self.backend.window_alive(lease.app_hwnd) {
            let from = self.backend.refresh(&lease.mon.gdi_name).map(|(_, r)| r).unwrap_or(lease.mon.rect);
            self.backend.return_window(lease.app_hwnd, from);
        }
    }

    fn release_locked(&self, g: &mut Inner, guid: &Guid) {
        let Some(lease) = g.leases.remove(guid) else { return };
        self.return_app_window(&lease);
        self.remove_lease(g, guid, &lease);
    }

    fn remove_lease(&self, g: &mut Inner, guid: &Guid, lease: &Lease) {
        match self.backend.remove(guid, Some(&lease.mon.gdi_name)) {
            Ok(()) => g.ledger.remove(guid),
            Err(e) => {
                tracing::warn!("could not remove virtual monitor {}: {e}; will retry", lease.mon.gdi_name);
                g.stuck.push((*guid, lease.mon.gdi_name.clone(), 1));
            }
        }
    }

    /// The phone went away: keep the monitor for `grace`, then remove it ([`reap`](Self::reap)).
    pub fn orphan(&self, guid: &Guid, now: Instant, grace: Duration) {
        if let Some(l) = self.inner.lock().unwrap().leases.get_mut(guid) {
            l.orphan_deadline = Some(now + grace);
            tracing::info!("virtual monitor {} kept for {} s", l.mon.gdi_name, grace.as_secs());
        }
    }

    /// A phone is back and attaches to this window: take over its kept monitor, if any.
    pub fn adopt(&self, app_hwnd: isize) -> Option<VMonitor> {
        let mut g = self.inner.lock().unwrap();
        let l = g.leases.values_mut().find(|l| l.orphan_deadline.is_some() && l.app_hwnd == app_hwnd)?;
        l.orphan_deadline = None;
        Some(l.mon.clone())
    }

    /// Remove kept monitors whose time is up or whose app window is gone.
    pub fn reap(&self, now: Instant) -> Vec<Guid> {
        let mut g = self.inner.lock().unwrap();
        let due: Vec<Guid> = g
            .leases
            .iter()
            .filter(|(_, l)| {
                l.orphan_deadline.is_some_and(|d| now >= d) || (l.orphan_deadline.is_some() && l.app_hwnd != 0 && !self.backend.window_alive(l.app_hwnd))
            })
            .map(|(k, _)| *k)
            .collect();
        for guid in &due {
            self.release_locked(&mut g, guid);
        }
        let stuck = std::mem::take(&mut g.stuck);
        for (guid, gdi, tries) in stuck {
            match self.backend.remove(&guid, Some(&gdi)) {
                Ok(()) => {
                    tracing::info!("virtual monitor {gdi} removed on retry {tries}");
                    g.ledger.remove(&guid);
                }
                Err(e) if tries < 20 => {
                    tracing::debug!("retry {tries} to remove {gdi} failed: {e}");
                    g.stuck.push((guid, gdi, tries + 1));
                }
                Err(e) => tracing::warn!("giving up removing virtual monitor {gdi}: {e}"),
            }
        }
        due
    }

    /// Number of monitors that currently exist (active or kept).
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().leases.len()
    }

    pub fn app_id_of(&self, guid: &Guid) -> Option<String> {
        self.inner.lock().unwrap().leases.get(guid).map(|l| l.app_id.clone())
    }

    /// Host exit: every monitor goes, windows return to the primary monitor.
    pub fn release_all(&self) {
        let mut g = self.inner.lock().unwrap();
        // Windows first (fast, and what the user sees), then the slower monitor removals: if the
        // process is killed half way, the driver's watchdog removes the rest.
        for lease in g.leases.values() {
            self.return_app_window(lease);
        }
        let all: Vec<(Guid, Lease)> = g.leases.drain().collect();
        for (guid, lease) in all {
            self.remove_lease(&mut g, &guid, &lease);
        }
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// A driver that only keeps notes.
    #[derive(Default)]
    pub struct FakeBackend {
        pub log: Mutex<Vec<String>>,
        pub monitors: Mutex<HashMap<String, Rect>>, // gdi -> rect
        pub fail_add: Mutex<bool>,
        pub allow_mode_change: Mutex<bool>,
        pub dead_windows: Mutex<Vec<isize>>,
        next: Mutex<u32>,
        gdi_of: Mutex<HashMap<Guid, String>>,
    }

    impl FakeBackend {
        pub fn real_monitors() -> Arc<FakeBackend> {
            let f = FakeBackend::default();
            f.monitors.lock().unwrap().insert(r"\\.\DISPLAY1".into(), Rect::new(0, 0, 2560, 1440));
            f.monitors.lock().unwrap().insert(r"\\.\DISPLAY2".into(), Rect::new(-1920, 0, 1920, 1080));
            Arc::new(f)
        }
        pub fn calls(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
        fn note(&self, s: String) {
            self.log.lock().unwrap().push(s);
        }
    }

    impl Backend for FakeBackend {
        fn add(&self, guid: &Guid, w: u32, h: u32, _r: u32) -> Result<AddedDisplay> {
            self.note(format!("add {w}x{h}"));
            if *self.fail_add.lock().unwrap() {
                return Err(anyhow!("driver says no"));
            }
            let gdi = self.gdi_of.lock().unwrap().entry(*guid).or_insert_with(|| {
                let mut n = self.next.lock().unwrap();
                *n += 1;
                format!(r"\\.\DISPLAY{}", 10 + *n)
            }).clone();
            let rect = Rect::new(7680, 0, w as i32, h as i32);
            self.monitors.lock().unwrap().insert(gdi.clone(), rect);
            Ok(AddedDisplay { gdi_name: gdi, hmonitor: 77, rect })
        }
        fn remove(&self, guid: &Guid, gdi: Option<&str>) -> Result<()> {
            self.note(format!("remove {}", guid.d1));
            let name = gdi.map(str::to_string).or_else(|| self.gdi_of.lock().unwrap().get(guid).cloned());
            if let Some(n) = name {
                if self.monitors.lock().unwrap().remove(&n).is_none() {
                    return Err(anyhow!("no such monitor"));
                }
            } else {
                return Err(anyhow!("no such monitor"));
            }
            Ok(())
        }
        fn other_monitor_rects(&self, except: &str) -> Vec<Rect> {
            self.monitors.lock().unwrap().iter().filter(|(k, _)| k.as_str() != except).map(|(_, v)| *v).collect()
        }
        fn position(&self, gdi: &str, x: i32, y: i32) -> Result<()> {
            self.note(format!("position {gdi} {x},{y}"));
            if let Some(r) = self.monitors.lock().unwrap().get_mut(gdi) {
                r.x = x;
                r.y = y;
            }
            Ok(())
        }
        fn set_dpi(&self, gdi: &str, wanted: u32) -> Result<u32> {
            self.note(format!("dpi {gdi} {wanted}"));
            // this fake offers up to 300%
            Ok(*[100u32, 125, 150, 175, 200, 225, 250, 300].iter().min_by_key(|v| ((**v).abs_diff(wanted), std::cmp::Reverse(**v))).unwrap())
        }
        fn refresh(&self, gdi: &str) -> Result<(isize, Rect)> {
            self.monitors.lock().unwrap().get(gdi).map(|r| (77, *r)).ok_or_else(|| anyhow!("gone"))
        }
        fn try_set_mode(&self, gdi: &str, w: u32, h: u32) -> bool {
            self.note(format!("mode {gdi} {w}x{h}"));
            if !*self.allow_mode_change.lock().unwrap() {
                return false;
            }
            if let Some(r) = self.monitors.lock().unwrap().get_mut(gdi) {
                r.w = w as i32;
                r.h = h as i32;
            }
            true
        }
        fn return_window(&self, hwnd: isize, _from: Rect) {
            self.note(format!("return {hwnd}"));
        }
        fn window_alive(&self, hwnd: isize) -> bool {
            !self.dead_windows.lock().unwrap().contains(&hwnd)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeBackend;
    use super::*;

    fn ledger(name: &str) -> (Ledger, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("penlink-vdm-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("vdisplays.json");
        (Ledger::load(&p), p)
    }

    #[test]
    fn create_places_right_of_real_monitors_sets_dpi_and_records_the_guid() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("create");
        let m = VdManager::new(be.clone(), l);
        let mon = m.create(1440, 3120, 275).unwrap();
        assert_eq!(mon.rect, Rect::new(2560, 0, 1440, 3120));
        assert_eq!((mon.dpi_wanted, mon.dpi_actual), (275, 300));
        assert!(mon.guid.is_penlink());
        let calls = be.calls();
        assert!(calls[0].starts_with("add 1440x3120"), "{calls:?}");
        assert!(calls.iter().any(|c| c.ends_with("2560,0")), "{calls:?}");
        // persisted for crash cleanup
        assert_eq!(Ledger::load(&p).all(), vec![mon.guid]);
        assert_eq!(m.count(), 1);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn second_monitor_goes_right_of_the_first() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("two");
        let m = VdManager::new(be, l);
        let a = m.create(1440, 3120, 300).unwrap();
        let b = m.create(720, 1560, 150).unwrap();
        assert_eq!(a.rect.x, 2560);
        assert_eq!(b.rect.x, 2560 + 1440);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn failed_add_leaves_nothing_behind() {
        let be = FakeBackend::real_monitors();
        *be.fail_add.lock().unwrap() = true;
        let (l, p) = ledger("fail");
        let m = VdManager::new(be, l);
        assert!(m.create(1440, 3120, 300).is_err());
        assert_eq!(m.count(), 0);
        assert!(Ledger::load(&p).all().is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn release_returns_the_window_then_removes_the_monitor() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("release");
        let m = VdManager::new(be.clone(), l);
        let mon = m.create(1440, 3120, 300).unwrap();
        m.bind(&mon.guid, 4242, "app");
        m.release(&mon.guid);
        let calls = be.calls();
        let ret = calls.iter().position(|c| c == "return 4242").unwrap();
        let rem = calls.iter().position(|c| c.starts_with("remove")).unwrap();
        assert!(ret < rem, "window must be moved before the monitor goes: {calls:?}");
        assert_eq!(m.count(), 0);
        assert!(Ledger::load(&p).all().is_empty());
        assert_eq!(be.monitors.lock().unwrap().len(), 2, "only the real monitors remain");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn start_up_removes_what_a_crashed_host_left() {
        let be = FakeBackend::real_monitors();
        let (mut l, p) = ledger("leftover");
        let g1 = Guid::penlink([1; 8]);
        let g2 = Guid::penlink([2; 8]);
        l.add(g1);
        l.add(g2);
        let m = VdManager::new(be.clone(), l);
        assert_eq!(m.cleanup_leftovers(), 2);
        assert_eq!(be.calls().iter().filter(|c| c.starts_with("remove")).count(), 2);
        assert!(Ledger::load(&p).all().is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn kept_monitor_is_adopted_before_the_grace_period_and_reaped_after() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("grace");
        let m = VdManager::new(be.clone(), l);
        let a = m.create(1440, 3120, 300).unwrap();
        m.bind(&a.guid, 100, "appA");
        let b = m.create(1440, 3120, 300).unwrap();
        m.bind(&b.guid, 200, "appB");
        let t0 = Instant::now();
        m.orphan(&a.guid, t0, GRACE);
        m.orphan(&b.guid, t0, GRACE);
        // nothing is due yet
        assert!(m.reap(t0 + GRACE - Duration::from_secs(1)).is_empty());
        // the phone is back for appA only
        let back = m.adopt(100).unwrap();
        assert_eq!(back.guid, a.guid);
        assert!(m.adopt(100).is_none(), "an adopted monitor is active again");
        assert!(m.adopt(999).is_none());
        // time passes: appB's monitor goes, appA's stays
        let gone = m.reap(t0 + GRACE + Duration::from_secs(1));
        assert_eq!(gone, vec![b.guid]);
        assert_eq!(m.count(), 1);
        assert!(m.get(&a.guid).is_some());
        assert!(be.calls().contains(&"return 200".to_string()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn kept_monitor_is_reaped_early_when_its_app_exits() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("early");
        let m = VdManager::new(be.clone(), l);
        let a = m.create(1440, 3120, 300).unwrap();
        m.bind(&a.guid, 100, "appA");
        let t0 = Instant::now();
        m.orphan(&a.guid, t0, GRACE);
        be.dead_windows.lock().unwrap().push(100);
        assert_eq!(m.reap(t0 + Duration::from_secs(5)), vec![a.guid]);
        // the window is dead: nothing to move
        assert!(!be.calls().iter().any(|c| c.starts_with("return")));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn active_monitors_are_never_reaped() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("active");
        let m = VdManager::new(be.clone(), l);
        let a = m.create(1440, 3120, 300).unwrap();
        m.bind(&a.guid, 100, "appA");
        be.dead_windows.lock().unwrap().push(100);
        assert!(m.reap(Instant::now() + Duration::from_secs(10_000)).is_empty());
        assert_eq!(m.count(), 1);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn reconfigure_switches_the_mode_when_windows_allows_it() {
        let be = FakeBackend::real_monitors();
        *be.allow_mode_change.lock().unwrap() = true;
        let (l, p) = ledger("mode");
        let m = VdManager::new(be.clone(), l);
        let a = m.create(1440, 3120, 300).unwrap();
        m.bind(&a.guid, 100, "appA");
        let b = m.reconfigure(&a, 3120, 1440, 300).unwrap();
        assert_eq!((b.rect.w, b.rect.h), (3120, 1440));
        assert_eq!(b.guid, a.guid);
        assert!(!be.calls().iter().any(|c| c.starts_with("remove")));
        assert_eq!(m.app_id_of(&a.guid).as_deref(), Some("appA"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn reconfigure_re_adds_under_the_same_guid_otherwise() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("readd");
        let m = VdManager::new(be.clone(), l);
        let a = m.create(1440, 3120, 300).unwrap();
        m.bind(&a.guid, 100, "appA");
        let b = m.reconfigure(&a, 3120, 1440, 250).unwrap();
        assert_eq!(b.guid, a.guid);
        assert_eq!((b.rect.w, b.rect.h), (3120, 1440));
        assert_eq!(b.rect.x, 2560, "back in the same place");
        assert_eq!(b.dpi_actual, 250);
        let calls = be.calls();
        assert!(calls.iter().any(|c| c.starts_with("remove")) && calls.iter().filter(|c| c.starts_with("add")).count() == 2);
        assert_eq!(Ledger::load(&p).all(), vec![a.guid]);
        assert_eq!(m.count(), 1);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn dpi_only_change() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("dpi");
        let m = VdManager::new(be, l);
        let a = m.create(1440, 3120, 300).unwrap();
        let b = m.set_dpi(&a, 200).unwrap();
        assert_eq!((b.dpi_wanted, b.dpi_actual), (200, 200));
        assert_eq!(m.get(&a.guid).unwrap().dpi_actual, 200);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn release_all_clears_everything() {
        let be = FakeBackend::real_monitors();
        let (l, p) = ledger("all");
        let m = VdManager::new(be.clone(), l);
        m.create(1440, 3120, 300).unwrap();
        m.create(720, 1560, 150).unwrap();
        m.release_all();
        assert_eq!(m.count(), 0);
        assert_eq!(be.monitors.lock().unwrap().len(), 2);
        assert!(Ledger::load(&p).all().is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
