//! Pure geometry: aspect-fit, input coordinate mapping, window sizing math.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
}

/// Largest rect with the aspect of `src` that fits in `dst`, centred; all values even so
/// 4:2:0 chroma lines up.
pub fn aspect_fit(src_w: i32, src_h: i32, dst_w: i32, dst_h: i32) -> Rect {
    if src_w <= 0 || src_h <= 0 || dst_w <= 0 || dst_h <= 0 {
        return Rect::new(0, 0, dst_w.max(0) & !1, dst_h.max(0) & !1);
    }
    // Compare dst_w/src_w with dst_h/src_h without floats.
    let (w, h) = if (dst_w as i64) * (src_h as i64) <= (dst_h as i64) * (src_w as i64) {
        // width-limited
        let h = ((dst_w as i64) * (src_h as i64) + (src_w as i64) / 2) / (src_w as i64);
        (dst_w as i64, h)
    } else {
        let w = ((dst_h as i64) * (src_w as i64) + (src_h as i64) / 2) / (src_h as i64);
        (w, dst_h as i64)
    };
    let w = ((w as i32).min(dst_w) & !1).max(2.min(dst_w & !1));
    let h = ((h as i32).min(dst_h) & !1).max(2.min(dst_h & !1));
    let x = ((dst_w - w) / 2) & !1;
    let y = ((dst_h - h) / 2) & !1;
    Rect::new(x, y, w, h)
}

/// Map normalised video coordinates (0..1 over the whole video frame, which contains the
/// client area aspect-fitted with black bars) to a point inside a client area of
/// `client_w` x `client_h` pixels. Result is relative to the client origin and clamped.
pub fn map_to_client(
    nx: f64,
    ny: f64,
    video_w: i32,
    video_h: i32,
    client_w: i32,
    client_h: i32,
) -> (i32, i32) {
    if client_w <= 0 || client_h <= 0 || video_w <= 0 || video_h <= 0 {
        return (0, 0);
    }
    let fit = aspect_fit(client_w, client_h, video_w, video_h);
    let vx = nx.clamp(0.0, 1.0) * video_w as f64;
    let vy = ny.clamp(0.0, 1.0) * video_h as f64;
    let fx = if fit.w > 0 { ((vx - fit.x as f64) / fit.w as f64).clamp(0.0, 1.0) } else { 0.0 };
    let fy = if fit.h > 0 { ((vy - fit.y as f64) / fit.h as f64).clamp(0.0, 1.0) } else { 0.0 };
    let px = (fx * (client_w - 1) as f64).round() as i32;
    let py = (fy * (client_h - 1) as f64).round() as i32;
    (px, py)
}

/// Client-area size in pixels for a requested video size and UI scale.
pub fn client_size_for(width: u32, height: u32, scale: f64) -> (i32, i32) {
    let s = if scale.is_finite() && scale >= 0.25 { scale } else { 1.0 };
    let w = (width as f64 / s).round().max(1.0) as i32;
    let h = (height as f64 / s).round().max(1.0) as i32;
    (w, h)
}

/// Shrink (never grow) `w` x `h` to fit in `max_w` x `max_h`, keeping the aspect ratio.
pub fn fit_within(w: i32, h: i32, max_w: i32, max_h: i32) -> (i32, i32) {
    if w <= 0 || h <= 0 || max_w <= 0 || max_h <= 0 || (w <= max_w && h <= max_h) {
        return (w, h);
    }
    let s = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    (((w as f64 * s).floor() as i32).max(1), ((h as f64 * s).floor() as i32).max(1))
}

/// Round a video dimension to an even number >= 2 (H.264 4:2:0).
pub fn even(v: u32) -> u32 {
    (v & !1).max(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_same_aspect_fills() {
        assert_eq!(aspect_fit(720, 1560, 1440, 3120), Rect::new(0, 0, 1440, 3120));
    }

    #[test]
    fn fit_wider_source_gets_letterbox() {
        let r = aspect_fit(1920, 1080, 1000, 1000);
        assert_eq!(r.w, 1000);
        assert_eq!(r.h, 562); // 1000*1080/1920 = 562.5 -> 563 -> even 562
        assert_eq!(r.x, 0);
        assert_eq!(r.y, (1000 - 562) / 2 & !1);
    }

    #[test]
    fn fit_taller_source_gets_pillarbox() {
        let r = aspect_fit(1000, 2000, 1000, 1000);
        assert_eq!(r.h, 1000);
        assert_eq!(r.w, 500);
        assert_eq!(r.x, 250);
        assert_eq!(r.y, 0);
    }

    #[test]
    fn fit_values_are_even_and_inside() {
        for (sw, sh, dw, dh) in [(333, 777, 1440, 3120), (1281, 721, 3120, 1440), (5, 3, 100, 100)] {
            let r = aspect_fit(sw, sh, dw, dh);
            assert!(r.x % 2 == 0 && r.y % 2 == 0 && r.w % 2 == 0 && r.h % 2 == 0, "{r:?}");
            assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= dw && r.y + r.h <= dh, "{r:?}");
        }
    }

    #[test]
    fn map_full_frame() {
        // same aspect: no bars
        assert_eq!(map_to_client(0.0, 0.0, 1440, 3120, 720, 1560), (0, 0));
        assert_eq!(map_to_client(1.0, 1.0, 1440, 3120, 720, 1560), (719, 1559));
        assert_eq!(map_to_client(0.5, 0.5, 1440, 3120, 720, 1560), (360, 780));
    }

    #[test]
    fn map_accounts_for_black_bars() {
        // client 1000x500 shown in a 1000x1000 video: fit = (0,250,1000,500)
        assert_eq!(map_to_client(0.5, 0.5, 1000, 1000, 1000, 500), (500, 250));
        assert_eq!(map_to_client(0.0, 0.25, 1000, 1000, 1000, 500), (0, 0));
        assert_eq!(map_to_client(1.0, 0.75, 1000, 1000, 1000, 500), (999, 499));
        // inside the bar clamps to the edge
        assert_eq!(map_to_client(0.5, 0.0, 1000, 1000, 1000, 500), (500, 0));
        assert_eq!(map_to_client(0.5, 1.0, 1000, 1000, 1000, 500), (500, 499));
    }

    #[test]
    fn map_clamps_out_of_range() {
        assert_eq!(map_to_client(-1.0, 5.0, 1440, 3120, 720, 1560), (0, 1559));
    }

    #[test]
    fn client_size() {
        assert_eq!(client_size_for(1440, 3120, 2.0), (720, 1560));
        assert_eq!(client_size_for(3120, 1440, 2.0), (1560, 720));
        assert_eq!(client_size_for(1000, 1000, 1.5), (667, 667));
        assert_eq!(client_size_for(1000, 1000, 0.0), (1000, 1000));
    }

    #[test]
    fn fit_within_bounds() {
        assert_eq!(fit_within(720, 1560, 1920, 1000), (461, 1000));
        assert_eq!(fit_within(100, 100, 1920, 1000), (100, 100));
    }

    #[test]
    fn even_rounding() {
        assert_eq!(even(1441), 1440);
        assert_eq!(even(0), 2);
        assert_eq!(even(3), 2);
    }
}
