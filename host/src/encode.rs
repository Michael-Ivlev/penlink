//! H.264 software encoder (openh264): constrained baseline, low-latency screen content.

use crate::frame::I420;
use anyhow::{anyhow, Result};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode, UsageType,
    VuiConfig,
};
use openh264::formats::YUVSlices;
use openh264::OpenH264API;

pub const DEFAULT_BITRATE: u32 = 10_000_000;
pub const MAX_FPS: f32 = 60.0;

#[derive(Debug, Clone, Copy)]
pub struct EncOptions {
    pub threads: u16,
    /// Max bytes per slice. openh264 only runs several threads when the picture is cut into
    /// slices; with one slice per frame `threads` has no effect.
    pub slice_len: Option<u32>,
    pub complexity: Complexity,
    /// ScreenContentRealTime (better text, slower) instead of CameraVideoRealTime.
    pub screen_content: bool,
    pub scene_detect: bool,
}

impl Default for EncOptions {
    fn default() -> Self {
        Self { threads: default_threads(), slice_len: Some(1200), complexity: Complexity::Low, screen_content: false, scene_detect: true }
    }
}

/// openh264 worker threads used by default (see `examples/encbench.rs` for the measurements).
///
/// Measured (examples/encbench.rs, 1440x3120, typing): single slice/thread + screen-content
/// mode 90 ms/frame (11 fps); 4 threads with 1200-byte slices + camera mode + low complexity
/// 32 ms/frame (31 fps). Slices are also what lets the RTP packetiser avoid fragmenting NALs.
pub fn default_threads() -> u16 {
    // PENLINK_ENC_THREADS: diagnostics / stress runs only.
    std::env::var("PENLINK_ENC_THREADS").ok().and_then(|v| v.parse::<u16>().ok()).filter(|n| (1..=16).contains(n)).unwrap_or(4)
}

pub struct H264Encoder {
    enc: Encoder,
    pub w: usize,
    pub h: usize,
}

pub struct Encoded {
    /// Annex-B access unit (SPS/PPS + slices).
    pub data: Vec<u8>,
    pub is_key: bool,
}

impl H264Encoder {
    pub fn new(w: usize, h: usize, bitrate: u32) -> Result<Self> {
        Self::with_threads(w, h, bitrate, default_threads())
    }

    /// `threads` = openh264 worker threads (1 = single threaded).
    pub fn with_threads(w: usize, h: usize, bitrate: u32, threads: u16) -> Result<Self> {
        Self::with_options(w, h, bitrate, EncOptions { threads, ..EncOptions::default() })
    }

    pub fn with_options(w: usize, h: usize, bitrate: u32, o: EncOptions) -> Result<Self> {
        let mut cfg = EncoderConfig::new()
            .num_threads(o.threads.max(1))
            .complexity(o.complexity)
            .usage_type(if o.screen_content { UsageType::ScreenContentRealTime } else { UsageType::CameraVideoRealTime })
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(bitrate))
            .max_frame_rate(FrameRate::from_hz(MAX_FPS))
            .profile(Profile::Baseline)
            .skip_frames(true)
            .scene_change_detect(o.scene_detect)
            .adaptive_quantization(false)
            .background_detection(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(0))
            .vui(VuiConfig::bt601());
        if let Some(n) = o.slice_len {
            cfg = cfg.max_slice_len(n);
        }
        let enc = Encoder::with_api_config(OpenH264API::from_source(), cfg)
            .map_err(|e| anyhow!("openh264 init: {e}"))?;
        Ok(Self { enc, w, h })
    }

    pub fn force_keyframe(&mut self) {
        self.enc.force_intra_frame();
    }

    pub fn encode(&mut self, f: &I420) -> Result<Option<Encoded>> {
        if f.w != self.w || f.h != self.h {
            return Err(anyhow!("frame size {}x{} != encoder {}x{}", f.w, f.h, self.w, self.h));
        }
        let src = YUVSlices::new((&f.y, &f.u, &f.v), (f.w, f.h), (f.w, f.w / 2, f.w / 2));
        let bs = self.enc.encode(&src).map_err(|e| anyhow!("openh264 encode: {e}"))?;
        let is_key = matches!(bs.frame_type(), openh264::encoder::FrameType::IDR | openh264::encoder::FrameType::I);
        if matches!(bs.frame_type(), openh264::encoder::FrameType::Skip | openh264::encoder::FrameType::Invalid) {
            return Ok(None);
        }
        let data = bs.to_vec();
        if data.is_empty() {
            return Ok(None);
        }
        Ok(Some(Encoded { data, is_key }))
    }
}

/// NAL unit types found in an Annex-B stream (for tests and diagnostics).
pub fn nal_types(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            out.push(data[i + 3] & 0x1f);
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Converter;
    use crate::geom::Rect;

    fn gradient_bgra(w: usize, h: usize, shift: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                v[o] = ((x + shift) % 256) as u8;
                v[o + 1] = (y % 256) as u8;
                v[o + 2] = ((x + y) % 256) as u8;
                v[o + 3] = 255;
            }
        }
        v
    }

    #[test]
    fn first_frame_is_idr_with_sps_pps_baseline() {
        let (w, h) = (320, 240);
        let mut enc = H264Encoder::new(w, h, 2_000_000).unwrap();
        let mut yuv = I420::new(w, h);
        let mut conv = Converter::new();
        conv.convert(&gradient_bgra(w, h, 0), w * 4, Rect::new(0, 0, w as i32, h as i32), &mut yuv);
        let e = enc.encode(&yuv).unwrap().expect("frame");
        assert!(e.is_key);
        let types = nal_types(&e.data);
        assert!(types.contains(&7), "SPS missing: {types:?}");
        assert!(types.contains(&8), "PPS missing: {types:?}");
        assert!(types.contains(&5), "IDR missing: {types:?}");
        // profile_idc 66 (baseline) follows the SPS NAL header
        let sps = e.data.windows(4).position(|w| w[0] == 0 && w[1] == 0 && w[2] == 1 && (w[3] & 0x1f) == 7).unwrap();
        assert_eq!(e.data[sps + 4], 66, "profile_idc");
    }

    #[test]
    fn keyframe_request_yields_idr_and_p_frames_follow() {
        let (w, h) = (320, 240);
        let mut enc = H264Encoder::new(w, h, 2_000_000).unwrap();
        let mut yuv = I420::new(w, h);
        let mut conv = Converter::new();
        let r = Rect::new(0, 0, w as i32, h as i32);
        conv.convert(&gradient_bgra(w, h, 0), w * 4, r, &mut yuv);
        enc.encode(&yuv).unwrap();
        conv.convert(&gradient_bgra(w, h, 3), w * 4, r, &mut yuv);
        let p = enc.encode(&yuv).unwrap().expect("p frame");
        assert!(!p.is_key);
        enc.force_keyframe();
        let k = enc.encode(&yuv).unwrap().expect("forced idr");
        assert!(k.is_key);
        assert!(nal_types(&k.data).contains(&5));
    }

    /// Every video size the phone can ask for must encode at every thread count (found by the
    /// stress run: some size / slice / thread combinations made openh264 return an error).
    #[test]
    fn all_sizes_and_thread_counts_encode() {
        let sizes = [
            (1440, 3120), (3120, 1440), (720, 1560), (1080, 2340), (1002, 776), (200, 400), (64, 64), (66, 66),
            (96, 64), (3840, 2160), (2160, 3840), (2560, 1080), (3840, 468), (468, 3840), (64, 3840),
        ];
        let mut failures = Vec::new();
        for threads in 1..=4u16 {
            for &(w, h) in &sizes {
                let mut enc = match H264Encoder::with_threads(w, h, DEFAULT_BITRATE, threads) {
                    Ok(e) => e,
                    Err(e) => {
                        failures.push(format!("{w}x{h} t{threads}: init {e}"));
                        continue;
                    }
                };
                let mut yuv = I420::new(w, h);
                let mut conv = Converter::new();
                for frame in 0..3 {
                    let src = gradient_bgra(w.min(640), h.min(640), frame * 5);
                    conv.convert(&src, w.min(640) * 4, Rect::new(0, 0, w.min(640) as i32, h.min(640) as i32), &mut yuv);
                    if frame == 2 {
                        enc.force_keyframe();
                    }
                    if let Err(e) = enc.encode(&yuv) {
                        failures.push(format!("{w}x{h} t{threads} frame {frame}: {e}"));
                        break;
                    }
                }
            }
        }
        assert!(failures.is_empty(), "encode failures:\n{}", failures.join("\n"));
    }

    #[test]
    fn wrong_size_is_rejected() {
        let mut enc = H264Encoder::new(64, 64, 500_000).unwrap();
        let yuv = I420::new(32, 32);
        assert!(enc.encode(&yuv).is_err());
    }
}
