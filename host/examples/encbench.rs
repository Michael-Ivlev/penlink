//! Software-encode benchmark: converts + encodes N BGRA frames of changing content.
//!
//!     cargo run --release --example encbench -- [frames] [threads...]
//!
//! For each size (1440x3120 and 1280x2856) and each scenario it prints the per-frame
//! convert and encode time, the achievable fps and the average frame size.
//! Scenarios: "typing" = a text-like page where a few glyph cells change per frame,
//! "scroll" = the whole page scrolls every frame (worst case).

use openh264::encoder::Complexity;
use penlink_host::encode::{EncOptions, H264Encoder, DEFAULT_BITRATE};
use penlink_host::frame::{Converter, I420};
use penlink_host::geom::Rect;
use std::time::Instant;

fn page(w: usize, h: usize) -> Vec<u8> {
    // Dark editor-like page: lines of pseudo text blocks.
    let mut v = vec![0u8; w * h * 4];
    let mut seed = 12345u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        seed >> 8
    };
    for px in v.chunks_exact_mut(4) {
        px.copy_from_slice(&[0x2e, 0x1e, 0x1e, 0xff]);
    }
    let line_h = 44;
    let mut y = 20;
    while y + line_h < h {
        let mut x = 40;
        while x + 30 < w - 40 {
            let len = 20 + (rnd() % 90) as usize;
            let col = [(rnd() % 200 + 55) as u8, (rnd() % 200 + 55) as u8, (rnd() % 200 + 55) as u8];
            for gx in (0..len).step_by(26) {
                if x + gx + 20 >= w {
                    break;
                }
                for yy in y..y + 28 {
                    for xx in (x + gx)..(x + gx + 18) {
                        let o = (yy * w + xx) * 4;
                        v[o..o + 4].copy_from_slice(&[col[2], col[1], col[0], 0xff]);
                    }
                }
            }
            x += len + 40;
        }
        y += line_h;
    }
    v
}

fn poke(buf: &mut [u8], w: usize, n: usize) {
    // change 6 glyph cells near a moving "caret"
    let y0 = 600 + (n / 40) * 44 % 1200;
    let x0 = 40 + (n % 40) * 26;
    for k in 0..6 {
        for yy in y0..y0 + 28 {
            for xx in (x0 + k * 26)..(x0 + k * 26 + 18) {
                let o = (yy * w + xx) * 4;
                let c = ((n * 7 + k * 31) % 200 + 55) as u8;
                buf[o..o + 4].copy_from_slice(&[c, 255 - c, c / 2, 0xff]);
            }
        }
    }
}

fn run(w: usize, h: usize, frames: usize, o: EncOptions, scenario: &str, base: &[u8]) {
    let threads = o.threads;
    let mut enc = H264Encoder::with_options(w, h, DEFAULT_BITRATE, o).unwrap();
    let mut conv = Converter::new();
    let mut yuv = I420::new(w, h);
    let mut buf = base.to_vec();
    let crop = Rect::new(0, 0, w as i32, h as i32);
    let (mut conv_t, mut enc_t, mut bytes, mut worst) = (0f64, 0f64, 0usize, 0f64);
    let mut encoded = 0;
    for n in 0..frames {
        match scenario {
            "typing" => poke(&mut buf, w, n),
            _ => {
                // scroll: shift every row up by 44 px (wrap), whole-frame change
                buf.rotate_left(w * 4 * 44);
            }
        }
        let t0 = Instant::now();
        conv.convert(&buf, w * 4, crop, &mut yuv);
        let t1 = Instant::now();
        let out = enc.encode(&yuv).unwrap();
        let t2 = Instant::now();
        let ct = (t1 - t0).as_secs_f64() * 1000.0;
        let et = (t2 - t1).as_secs_f64() * 1000.0;
        conv_t += ct;
        enc_t += et;
        worst = f64::max(worst, ct + et);
        if let Some(o) = out {
            bytes += o.data.len();
            encoded += 1;
        }
    }
    let n = frames as f64;
    println!(
        "{w}x{h} {scenario:<6} threads={threads:<2} slice={:<5} cx={:<6} screen={:<5} sc={:<5} convert {:5.1} ms  encode {:5.1} ms  total {:5.1} ms/frame -> {:5.1} fps  (worst {:5.1} ms, {:.0} KB/frame, {} frames kept)",
        o.slice_len.map(|s| s.to_string()).unwrap_or("-".into()),
        format!("{:?}", o.complexity),
        o.screen_content,
        o.scene_detect,
        conv_t / n,
        enc_t / n,
        (conv_t + enc_t) / n,
        1000.0 / ((conv_t + enc_t) / n),
        worst,
        if encoded > 0 { bytes as f64 / encoded as f64 / 1024.0 } else { 0.0 },
        encoded
    );
}

fn main() {
    // usage: encbench [frames] [threads:slice:complexity[:cam][:nosc] ...]   e.g. 300 1:0:m 4:1200:l:cam:nosc
    let mut args = std::env::args().skip(1);
    let frames: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(300);
    let mut opts: Vec<EncOptions> = args
        .map(|a| {
            let p: Vec<&str> = a.split(':').collect();
            EncOptions {
                threads: p.first().and_then(|x| x.parse().ok()).unwrap_or(1),
                slice_len: p.get(1).and_then(|x| x.parse().ok()).filter(|&n: &u32| n > 0),
                complexity: match p.get(2).copied() {
                    Some("l") => Complexity::Low,
                    Some("h") => Complexity::High,
                    _ => Complexity::Medium,
                },
                screen_content: p.get(3).copied() != Some("cam"),
                scene_detect: p.get(4).copied() != Some("nosc"),
            }
        })
        .collect();
    if opts.is_empty() {
        opts.push(EncOptions::default());
    }
    println!("cpu threads available: {}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
    for (w, h) in [(1440usize, 3120usize), (1280, 2856)] {
        let base = page(w, h);
        for sc in ["typing", "scroll"] {
            for &o in &opts {
                run(w, h, frames, o, sc, &base);
            }
        }
    }
}
