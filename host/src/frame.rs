//! BGRA -> I420 (BT.601 limited range) with crop and aspect-fit scaling into the output
//! frame (black bars). Multithreaded over row bands, nearest-neighbour sampling.

use crate::geom::{aspect_fit, Rect};

pub struct I420 {
    pub w: usize,
    pub h: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl I420 {
    /// `w` and `h` must be even.
    pub fn new(w: usize, h: usize) -> Self {
        assert!(w % 2 == 0 && h % 2 == 0 && w > 0 && h > 0, "I420 size must be even");
        let mut f = Self { w, h, y: vec![16; w * h], u: vec![128; w * h / 4], v: vec![128; w * h / 4] };
        f.fill_black();
        f
    }

    pub fn fill_black(&mut self) {
        self.y.fill(16);
        self.u.fill(128);
        self.v.fill(128);
    }
}

#[derive(Default)]
pub struct Converter {
    key: Option<(usize, usize, Rect)>,
    fit: Option<Rect>,
    xmap: Vec<usize>, // byte offset of the source pixel for each dst column inside `fit`
    ymap: Vec<usize>, // source row for each dst row inside `fit`
}

#[inline(always)]
fn yuv(r: i32, g: i32, b: i32) -> u8 {
    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8
}

impl Converter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert the `crop` region of a BGRA image (`stride` bytes per row) into `dst`.
    /// Returns the rect of `dst` that holds image content.
    pub fn convert(&mut self, src: &[u8], stride: usize, crop: Rect, dst: &mut I420) -> Rect {
        let key = (dst.w, dst.h, crop);
        if self.key != Some(key) {
            let fit = aspect_fit(crop.w, crop.h, dst.w as i32, dst.h as i32);
            self.xmap = (0..fit.w)
                .map(|i| {
                    let sx = ((2 * i as i64 + 1) * crop.w as i64) / (2 * fit.w as i64);
                    (crop.x as usize + sx as usize) * 4
                })
                .collect();
            self.ymap = (0..fit.h)
                .map(|i| {
                    let sy = ((2 * i as i64 + 1) * crop.h as i64) / (2 * fit.h as i64);
                    crop.y as usize + sy as usize
                })
                .collect();
            if self.fit != Some(fit) || self.key.map(|k| (k.0, k.1)) != Some((dst.w, dst.h)) {
                dst.fill_black();
            }
            self.fit = Some(fit);
            self.key = Some(key);
        }
        let fit = self.fit.unwrap();
        if fit.w < 2 || fit.h < 2 {
            return fit;
        }
        let (w, cw) = (dst.w, dst.w / 2);
        let pairs = (fit.h / 2) as usize;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 6);
        // Small frames are not worth threading.
        let threads = if (fit.w as usize * fit.h as usize) < 200_000 { 1 } else { threads };
        let per = pairs.div_ceil(threads);

        let y_sub = &mut dst.y[fit.y as usize * w..(fit.y + fit.h) as usize * w];
        let u_sub = &mut dst.u[(fit.y as usize / 2) * cw..((fit.y + fit.h) as usize / 2) * cw];
        let v_sub = &mut dst.v[(fit.y as usize / 2) * cw..((fit.y + fit.h) as usize / 2) * cw];
        let xmap = &self.xmap;
        let ymap = &self.ymap;

        std::thread::scope(|s| {
            for (i, ((yc, uc), vc)) in y_sub
                .chunks_mut(per * 2 * w)
                .zip(u_sub.chunks_mut(per * cw))
                .zip(v_sub.chunks_mut(per * cw))
                .enumerate()
            {
                let p0 = i * per;
                let mut job = move || convert_band(src, stride, fit, xmap, ymap, p0, yc, uc, vc, w);
                if threads == 1 {
                    job();
                } else {
                    s.spawn(job);
                }
            }
        });
        fit
    }
}

#[allow(clippy::too_many_arguments)]
fn convert_band(
    src: &[u8],
    stride: usize,
    fit: Rect,
    xmap: &[usize],
    ymap: &[usize],
    p0: usize,
    yc: &mut [u8],
    uc: &mut [u8],
    vc: &mut [u8],
    w: usize,
) {
    let cw = w / 2;
    let fx = fit.x as usize;
    let fw = fit.w as usize;
    let pairs = uc.len() / cw;
    for p in 0..pairs {
        let r0 = ymap[(p0 + p) * 2] * stride;
        let r1 = ymap[(p0 + p) * 2 + 1] * stride;
        let (y_top, y_bot) = yc[p * 2 * w..(p * 2 + 2) * w].split_at_mut(w);
        let y_top = &mut y_top[fx..fx + fw];
        let y_bot = &mut y_bot[fx..fx + fw];
        let u_row = &mut uc[p * cw + fx / 2..p * cw + fx / 2 + fw / 2];
        let v_row = &mut vc[p * cw + fx / 2..p * cw + fx / 2 + fw / 2];
        for c in 0..fw / 2 {
            let a = r0 + xmap[c * 2];
            let b = r0 + xmap[c * 2 + 1];
            let cc = r1 + xmap[c * 2];
            let d = r1 + xmap[c * 2 + 1];
            let (b0, g0, r0v) = (src[a] as i32, src[a + 1] as i32, src[a + 2] as i32);
            let (b1, g1, r1v) = (src[b] as i32, src[b + 1] as i32, src[b + 2] as i32);
            let (b2, g2, r2v) = (src[cc] as i32, src[cc + 1] as i32, src[cc + 2] as i32);
            let (b3, g3, r3v) = (src[d] as i32, src[d + 1] as i32, src[d + 2] as i32);
            y_top[c * 2] = yuv(r0v, g0, b0);
            y_top[c * 2 + 1] = yuv(r1v, g1, b1);
            y_bot[c * 2] = yuv(r2v, g2, b2);
            y_bot[c * 2 + 1] = yuv(r3v, g3, b3);
            let rs = r0v + r1v + r2v + r3v;
            let gs = g0 + g1 + g2 + g3;
            let bs = b0 + b1 + b2 + b3;
            u_row[c] = (((-38 * rs - 74 * gs + 112 * bs + 512) >> 10) + 128).clamp(0, 255) as u8;
            v_row[c] = (((112 * rs - 94 * gs - 18 * bs + 512) >> 10) + 128).clamp(0, 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, b: u8, g: u8, r: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            v.extend_from_slice(&[b, g, r, 255]);
        }
        v
    }

    #[test]
    fn red_converts_to_expected_yuv() {
        let src = solid(64, 64, 0, 0, 255);
        let mut dst = I420::new(64, 64);
        let mut c = Converter::new();
        let fit = c.convert(&src, 64 * 4, Rect::new(0, 0, 64, 64), &mut dst);
        assert_eq!(fit, Rect::new(0, 0, 64, 64));
        assert_eq!(dst.y[0], 82); // (66*255+128>>8)+16
        assert!((dst.u[0] as i32 - 90).abs() <= 1, "u={}", dst.u[0]);
        assert!((dst.v[0] as i32 - 240).abs() <= 1, "v={}", dst.v[0]);
    }

    #[test]
    fn white_and_black_are_limited_range() {
        let mut dst = I420::new(8, 8);
        let mut c = Converter::new();
        c.convert(&solid(8, 8, 255, 255, 255), 32, Rect::new(0, 0, 8, 8), &mut dst);
        assert_eq!(dst.y[0], 235);
        assert!((dst.u[0] as i32 - 128).abs() <= 1);
        c.convert(&solid(8, 8, 0, 0, 0), 32, Rect::new(0, 0, 8, 8), &mut dst);
        assert_eq!(dst.y[0], 16);
    }

    #[test]
    fn letterbox_bars_stay_black() {
        // 40x20 white image into a 40x40 frame -> 20 rows of bars split top/bottom
        let src = solid(40, 20, 255, 255, 255);
        let mut dst = I420::new(40, 40);
        let mut c = Converter::new();
        let fit = c.convert(&src, 40 * 4, Rect::new(0, 0, 40, 20), &mut dst);
        assert_eq!(fit, Rect::new(0, 10, 40, 20));
        assert_eq!(dst.y[0], 16); // top bar
        assert_eq!(dst.y[39 * 40], 16); // bottom bar
        assert_eq!(dst.y[20 * 40 + 5], 235); // image
        assert_eq!(dst.u[0], 128);
    }

    #[test]
    fn crop_selects_region_and_scales_up() {
        // left half red, right half blue; crop only the right half, scale 2x
        let (w, h) = (32usize, 16usize);
        let mut src = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                if x < 16 {
                    src.extend_from_slice(&[0, 0, 255, 255]);
                } else {
                    src.extend_from_slice(&[255, 0, 0, 255]);
                }
            }
        }
        let mut dst = I420::new(32, 32);
        let mut c = Converter::new();
        let fit = c.convert(&src, w * 4, Rect::new(16, 0, 16, 16), &mut dst);
        assert_eq!(fit, Rect::new(0, 0, 32, 32));
        // blue: Y=41, U=240, V=110
        assert_eq!(dst.y[0], 41);
        assert!((dst.u[0] as i32 - 240).abs() <= 1);
        assert!((dst.v[0] as i32 - 110).abs() <= 1);
    }

    #[test]
    fn large_multithreaded_matches_single_thread_layout() {
        // gradient so rows differ; check a few sample points map to the right source rows
        let (w, h) = (800usize, 600usize);
        let mut src = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                src[o] = 0;
                src[o + 1] = (y % 256) as u8;
                src[o + 2] = 0;
                src[o + 3] = 255;
            }
        }
        let mut dst = I420::new(800, 600);
        let mut c = Converter::new();
        c.convert(&src, w * 4, Rect::new(0, 0, 800, 600), &mut dst);
        for y in [0usize, 1, 100, 299, 300, 598, 599] {
            let g = (y % 256) as i32;
            let expect = (((129 * g + 128) >> 8) + 16) as u8;
            assert_eq!(dst.y[y * 800 + 7], expect, "row {y}");
        }
    }
}
