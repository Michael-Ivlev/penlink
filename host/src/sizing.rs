//! Pure rules for virtual monitors: how big the monitor is, which DPI step it gets, where it
//! is placed, what a `resize` does to it, and which foreground window may receive input.
//! No Win32 here, so everything is unit tested.

use crate::geom::{clamp_video, even, Rect};

// ---------------------------------------------------------------------------------------
// Quality / UI scale

/// `sharp`: the monitor has the phone's pixel size. `fast`: half of it (the video is upscaled
/// on the phone: bigger UI, lighter stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    #[default]
    Sharp,
    Fast,
}

impl Quality {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sharp" => Some(Quality::Sharp),
            "fast" => Some(Quality::Fast),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Sharp => "sharp",
            Quality::Fast => "fast",
        }
    }
}

/// Default UI size in percent when a client sends `quality` but no `uiScale`.
pub const DEFAULT_UI_SCALE: f64 = 300.0;
pub const MIN_UI_SCALE: f64 = 100.0;
pub const MAX_UI_SCALE: f64 = 500.0;

/// What the phone asked for (`open` / `resize`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeRequest {
    pub width: u32,
    pub height: u32,
    pub quality: Quality,
    /// UI size in percent (100 = Windows' 100%).
    pub ui_scale: f64,
}

impl SizeRequest {
    /// Build from the wire fields. Old clients send only `scale` (a factor, 2.0 = 200%).
    pub fn from_wire(width: u32, height: u32, scale: f64, quality: Option<&str>, ui_scale: Option<f64>) -> Self {
        let ui = ui_scale
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(if scale.is_finite() && scale > 0.0 { scale * 100.0 } else { 100.0 });
        Self {
            width,
            height,
            quality: quality.and_then(Quality::parse).unwrap_or_default(),
            ui_scale: ui.clamp(MIN_UI_SCALE, MAX_UI_SCALE),
        }
    }

    /// A client that sends `quality` or `uiScale` understands virtual monitors.
    pub fn wants_monitor(quality: Option<&str>, ui_scale: Option<f64>) -> bool {
        quality.is_some() || ui_scale.is_some()
    }
}

/// The sizes derived from a request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizePlan {
    /// Video size = virtual monitor size (even, inside the encoder box).
    pub video_w: u32,
    pub video_h: u32,
    /// DPI the monitor should get, in percent (sharp: the UI size; fast: half of it).
    pub dpi_pct: u32,
}

impl SizePlan {
    /// Window-capture fallback: client area = video / (dpi / 100), so the picture matches
    /// the monitor case (same UI size relative to the phone screen).
    pub fn window_scale(&self) -> f64 {
        (self.dpi_pct as f64 / 100.0).max(0.25)
    }
}

pub fn plan_sizes(req: &SizeRequest) -> SizePlan {
    let (w, h, dpi) = match req.quality {
        Quality::Sharp => (req.width, req.height, req.ui_scale),
        Quality::Fast => ((req.width + 1) / 2, (req.height + 1) / 2, req.ui_scale / 2.0),
    };
    let (vw, vh) = clamp_video(w.max(2), h.max(2));
    SizePlan { video_w: vw, video_h: vh, dpi_pct: dpi.round().max(1.0) as u32 }
}

// ---------------------------------------------------------------------------------------
// DPI steps (undocumented DisplayConfig types -3 / -4, see SPEC)

/// DPI percentages Windows offers, in order. The OS reports scale values relative to the
/// recommended one: `rel = index - recommended_index`.
pub const DPI_STEPS: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

/// Answer of `DISPLAYCONFIG_DEVICE_INFO_GET_DPI_SCALE` (-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiRange {
    pub min_rel: i32,
    pub cur_rel: i32,
    pub max_rel: i32,
}

impl DpiRange {
    /// Index of the recommended step in [`DPI_STEPS`] (100% is always the minimum).
    pub fn recommended_index(&self) -> i32 {
        -self.min_rel.min(0)
    }

    fn index_of_rel(&self, rel: i32) -> Option<usize> {
        let i = self.recommended_index() + rel;
        (i >= 0 && (i as usize) < DPI_STEPS.len()).then_some(i as usize)
    }

    pub fn current_percent(&self) -> Option<u32> {
        let rel = self.cur_rel.clamp(self.min_rel, self.max_rel);
        self.index_of_rel(rel).map(|i| DPI_STEPS[i])
    }

    pub fn recommended_percent(&self) -> Option<u32> {
        self.index_of_rel(0).map(|i| DPI_STEPS[i])
    }

    /// Every percentage the OS allows for this monitor.
    pub fn allowed(&self) -> Vec<u32> {
        (self.min_rel..=self.max_rel).filter_map(|r| self.index_of_rel(r)).map(|i| DPI_STEPS[i]).collect()
    }
}

/// The allowed step nearest to `wanted` percent (ties go up: bigger buttons) and the
/// relative value to hand to `SET_DPI_SCALE` (-4). `None` when the range is unusable.
pub fn pick_dpi_step(wanted: u32, range: &DpiRange) -> Option<(i32, u32)> {
    // The recommended step must be one we know (the reference code gives up here too).
    if range.min_rel > 0 || range.recommended_index() as usize >= DPI_STEPS.len() {
        return None;
    }
    let mut best: Option<(i32, u32)> = None;
    for rel in range.min_rel..=range.max_rel {
        let Some(i) = range.index_of_rel(rel) else { continue };
        let pct = DPI_STEPS[i];
        let better = match best {
            None => true,
            Some((_, b)) => {
                let (d, db) = (pct.abs_diff(wanted), b.abs_diff(wanted));
                d < db || (d == db && pct > b)
            }
        };
        if better {
            best = Some((rel, pct));
        }
    }
    best
}

// ---------------------------------------------------------------------------------------
// Placement

/// Top-left for a new monitor: right of everything that exists now, top-aligned with the
/// topmost monitor. (Never the primary: positions are relative to the primary at 0,0.)
pub fn place_right_of(existing: &[Rect]) -> (i32, i32) {
    if existing.is_empty() {
        return (0, 0);
    }
    let x = existing.iter().map(|r| r.x + r.w).max().unwrap_or(0);
    let y = existing.iter().map(|r| r.y).min().unwrap_or(0);
    (x, y)
}

/// Where a window goes when its virtual monitor disappears and it must live on the primary
/// monitor: keep its size if it fits the work area, otherwise shrink it; place it inside,
/// at a small offset from the corner.
pub fn evacuation_rect(win: Rect, work: Rect) -> Rect {
    let margin = 40;
    let w = win.w.min(work.w).max(1);
    let h = win.h.min(work.h).max(1);
    let x = (work.x + margin).min(work.x + work.w - w).max(work.x);
    let y = (work.y + margin).min(work.y + work.h - h).max(work.y);
    Rect::new(x, y, w, h)
}

pub fn rect_contains(outer: Rect, p: (i32, i32)) -> bool {
    p.0 >= outer.x && p.1 >= outer.y && p.0 < outer.x + outer.w && p.1 < outer.y + outer.h
}

pub fn rect_center(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

pub fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Input is accepted when the foreground window belongs to the session's app (its own process,
/// or owned by the app window: dialogs and popups) and sits on the session's monitor.
pub fn foreground_accepts(fg_in_session: bool, fg_rect: Option<Rect>, monitor: Rect) -> bool {
    fg_in_session && fg_rect.is_some_and(|r| rect_contains(monitor, rect_center(r)))
}

// ---------------------------------------------------------------------------------------
// Resize

/// The monitor as it currently is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorSpec {
    pub w: u32,
    pub h: u32,
    /// DPI percentage that was asked for when it was configured.
    pub dpi_wanted: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeAction {
    /// Different width, or taller than the monitor (rotation, quality change): new monitor size.
    Reconfigure { w: u32, h: u32 },
    /// Same monitor. `view_h` rows (from the top) are streamed: smaller than the monitor while
    /// the phone keyboard is up. `dpi_changed`: apply the new DPI step.
    InPlace { view_h: u32, dpi_changed: bool },
}

pub fn decide_resize(cur: MonitorSpec, plan: &SizePlan) -> ResizeAction {
    if plan.video_w != cur.w || plan.video_h > cur.h {
        return ResizeAction::Reconfigure { w: plan.video_w, h: plan.video_h };
    }
    ResizeAction::InPlace { view_h: even(plan.video_h), dpi_changed: plan.dpi_pct != cur.dpi_wanted }
}

// ---------------------------------------------------------------------------------------
// Render adapter

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    pub description: String,
    pub vendor_id: u32,
    pub dedicated_vram: u64,
    /// "Microsoft Basic Render Driver" and friends.
    pub software: bool,
}

/// GPU the virtual monitors render on: the one named by `preferred` (substring, case
/// insensitive) if any, else the first hardware adapter, which is what D3D11's default
/// device (used by the capture) is, so no copy between GPUs is needed.
pub fn pick_render_adapter(list: &[AdapterInfo], preferred: Option<&str>) -> Option<usize> {
    if let Some(p) = preferred.map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()) {
        if let Some(i) = list.iter().position(|a| !a.software && a.description.to_ascii_lowercase().contains(&p)) {
            return Some(i);
        }
    }
    list.iter().position(|a| !a.software)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(w: u32, h: u32, q: Quality, ui: f64) -> SizeRequest {
        SizeRequest { width: w, height: h, quality: q, ui_scale: ui }
    }

    #[test]
    fn sharp_monitor_is_phone_sized_and_dpi_is_ui_scale() {
        let p = plan_sizes(&req(1440, 3120, Quality::Sharp, 300.0));
        assert_eq!(p, SizePlan { video_w: 1440, video_h: 3120, dpi_pct: 300 });
    }

    #[test]
    fn fast_monitor_is_half_size_and_half_dpi() {
        let p = plan_sizes(&req(1440, 3120, Quality::Fast, 300.0));
        assert_eq!(p, SizePlan { video_w: 720, video_h: 1560, dpi_pct: 150 });
        // odd sizes round up then to even
        let p = plan_sizes(&req(1281, 2857, Quality::Fast, 250.0));
        assert_eq!((p.video_w, p.video_h, p.dpi_pct), (640, 1428, 125));
    }

    #[test]
    fn oversized_phone_is_clamped_to_the_encoder_box() {
        let p = plan_sizes(&req(4320, 9360, Quality::Sharp, 300.0));
        assert!(p.video_w <= 2160 && p.video_h <= 3840 && p.video_w % 2 == 0 && p.video_h % 2 == 0);
        // aspect kept
        let a = 4320.0 / 9360.0;
        assert!((p.video_w as f64 / p.video_h as f64 - a).abs() < 0.01);
    }

    #[test]
    fn window_scale_matches_the_monitor_case() {
        // sharp: client = 1440/3.0 ; fast: client = 720 / 1.5 = same window
        let s = plan_sizes(&req(1440, 3120, Quality::Sharp, 300.0));
        let f = plan_sizes(&req(1440, 3120, Quality::Fast, 300.0));
        let cs = crate::geom::client_size_for(s.video_w, s.video_h, s.window_scale());
        let cf = crate::geom::client_size_for(f.video_w, f.video_h, f.window_scale());
        assert_eq!(cs, cf);
        assert_eq!(cs, (480, 1040));
    }

    #[test]
    fn old_clients_scale_becomes_ui_percent() {
        let r = SizeRequest::from_wire(1440, 3120, 2.0, None, None);
        assert_eq!((r.quality, r.ui_scale), (Quality::Sharp, 200.0));
        let r = SizeRequest::from_wire(1440, 3120, 2.0, Some("fast"), Some(275.0));
        assert_eq!((r.quality, r.ui_scale), (Quality::Fast, 275.0));
        // out of range / junk is clamped or ignored
        assert_eq!(SizeRequest::from_wire(1, 1, 1.0, Some("weird"), Some(9000.0)).ui_scale, 500.0);
        assert_eq!(SizeRequest::from_wire(1, 1, 1.0, Some("weird"), Some(9000.0)).quality, Quality::Sharp);
        assert_eq!(SizeRequest::from_wire(1, 1, f64::NAN, None, None).ui_scale, 100.0);
        assert_eq!(SizeRequest::from_wire(1, 1, 1.0, None, Some(10.0)).ui_scale, 100.0);
    }

    #[test]
    fn monitor_only_for_clients_that_send_the_new_fields() {
        assert!(!SizeRequest::wants_monitor(None, None));
        assert!(SizeRequest::wants_monitor(Some("fast"), None));
        assert!(SizeRequest::wants_monitor(None, Some(250.0)));
    }

    fn range(min: i32, cur: i32, max: i32) -> DpiRange {
        DpiRange { min_rel: min, cur_rel: cur, max_rel: max }
    }

    #[test]
    fn dpi_range_maps_relative_values() {
        // recommended 175% (index 3), 100% is 3 steps down, max +4 = 250%... index 7 = 300
        let r = range(-3, 0, 4);
        assert_eq!(r.recommended_index(), 3);
        assert_eq!(r.recommended_percent(), Some(175));
        assert_eq!(r.current_percent(), Some(175));
        assert_eq!(r.allowed(), vec![100, 125, 150, 175, 200, 225, 250, 300]);
        // current outside the range is clamped like the reference implementation
        assert_eq!(range(-3, 9, 4).current_percent(), Some(300));
    }

    #[test]
    fn nearest_step_prefers_bigger_on_ties() {
        let r = range(-3, 0, 4); // up to 300
        assert_eq!(pick_dpi_step(250, &r), Some((3, 250))); // 250 is index 6, recommended 3
        assert_eq!(pick_dpi_step(275, &r), Some((4, 300))); // tie 250/300 -> 300
        assert_eq!(pick_dpi_step(100, &r), Some((-3, 100)));
        assert_eq!(pick_dpi_step(50, &r), Some((-3, 100)));
        // above the maximum: the maximum
        assert_eq!(pick_dpi_step(400, &r), Some((4, 300)));
        assert_eq!(pick_dpi_step(130, &r), Some((-2, 125)));
    }

    #[test]
    fn dpi_range_unusable_gives_none() {
        // recommended index beyond the table
        assert_eq!(pick_dpi_step(100, &range(-20, 0, 0)), None);
    }

    #[test]
    fn placement_goes_right_of_everything() {
        let real = [Rect::new(0, 0, 2560, 1440), Rect::new(-1920, 100, 1920, 1080)];
        assert_eq!(place_right_of(&real), (2560, 0));
        assert_eq!(place_right_of(&[]), (0, 0));
        // second virtual monitor stacks further right
        let mut v = real.to_vec();
        v.push(Rect::new(2560, 0, 1440, 3120));
        assert_eq!(place_right_of(&v), (4000, 0));
    }

    #[test]
    fn evacuation_keeps_size_when_it_fits_and_shrinks_otherwise() {
        let work = Rect::new(0, 0, 2560, 1400);
        assert_eq!(evacuation_rect(Rect::new(9000, 0, 480, 1040), work), Rect::new(40, 40, 480, 1040));
        let r = evacuation_rect(Rect::new(9000, 0, 1440, 3120), work);
        assert_eq!((r.w, r.h), (1440, 1400));
        assert!(r.y == 0 && r.x + r.w <= 2560);
        // work area not at the origin
        let work = Rect::new(0, 30, 1920, 1010);
        let r = evacuation_rect(Rect::new(5000, 5000, 100, 100), work);
        assert!(r.x >= 0 && r.y >= 30 && r.x + r.w <= 1920 && r.y + r.h <= 1040);
    }

    #[test]
    fn foreground_must_be_the_apps_and_on_the_monitor() {
        let mon = Rect::new(2560, 0, 1440, 3120);
        let on = Rect::new(2560, 0, 1440, 3120);
        let popup = Rect::new(3000, 800, 400, 300);
        let off = Rect::new(100, 100, 800, 600);
        assert!(foreground_accepts(true, Some(on), mon));
        assert!(foreground_accepts(true, Some(popup), mon)); // dialog on the monitor
        assert!(!foreground_accepts(true, Some(off), mon)); // app window moved away
        assert!(!foreground_accepts(false, Some(on), mon)); // some other app's window
        assert!(!foreground_accepts(true, None, mon));
    }

    #[test]
    fn rect_helpers() {
        let a = Rect::new(0, 0, 10, 10);
        assert!(rects_intersect(a, Rect::new(9, 9, 5, 5)));
        assert!(!rects_intersect(a, Rect::new(10, 0, 5, 5)));
        assert!(rect_contains(a, (0, 0)) && !rect_contains(a, (10, 5)));
    }

    #[test]
    fn resize_decisions() {
        let cur = MonitorSpec { w: 1440, h: 3120, dpi_wanted: 300 };
        let plan = |w, h, dpi| SizePlan { video_w: w, video_h: h, dpi_pct: dpi };
        // same request: nothing to do
        assert_eq!(decide_resize(cur, &plan(1440, 3120, 300)), ResizeAction::InPlace { view_h: 3120, dpi_changed: false });
        // keyboard up: shorter, same width -> view only
        assert_eq!(decide_resize(cur, &plan(1440, 1500, 300)), ResizeAction::InPlace { view_h: 1500, dpi_changed: false });
        // keyboard down again
        assert_eq!(decide_resize(cur, &plan(1440, 3120, 300)), ResizeAction::InPlace { view_h: 3120, dpi_changed: false });
        // UI size slider
        assert_eq!(decide_resize(cur, &plan(1440, 3120, 250)), ResizeAction::InPlace { view_h: 3120, dpi_changed: true });
        // rotation
        assert_eq!(decide_resize(cur, &plan(3120, 1440, 300)), ResizeAction::Reconfigure { w: 3120, h: 1440 });
        // taller than the monitor
        assert_eq!(decide_resize(cur, &plan(1440, 3200, 300)), ResizeAction::Reconfigure { w: 1440, h: 3200 });
        // sharp -> fast
        assert_eq!(decide_resize(cur, &plan(720, 1560, 150)), ResizeAction::Reconfigure { w: 720, h: 1560 });
    }

    fn ad(name: &str, software: bool) -> AdapterInfo {
        AdapterInfo { description: name.into(), vendor_id: 0x8086, dedicated_vram: 1 << 30, software }
    }

    #[test]
    fn render_adapter_choice() {
        let l = [ad("Microsoft Basic Render Driver", true), ad("Intel(R) Arc(TM) B580 Graphics", false), ad("Intel UHD", false)];
        assert_eq!(pick_render_adapter(&l, None), Some(1));
        assert_eq!(pick_render_adapter(&l, Some("uhd")), Some(2));
        assert_eq!(pick_render_adapter(&l, Some("nvidia")), Some(1));
        assert_eq!(pick_render_adapter(&l[..1], None), None);
        assert_eq!(pick_render_adapter(&[], None), None);
    }
}
