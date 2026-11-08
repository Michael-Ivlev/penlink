//! H.264 software encoder (openh264): constrained baseline, low-latency screen content.

use crate::frame::I420;
use anyhow::{anyhow, Result};
use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode, UsageType,
    VuiConfig,
};
use openh264::formats::YUVSlices;
use openh264::OpenH264API;

pub const DEFAULT_BITRATE: u32 = 10_000_000;
pub const MAX_FPS: f32 = 60.0;

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
        let cfg = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(bitrate))
            .max_frame_rate(FrameRate::from_hz(MAX_FPS))
            .profile(Profile::Baseline)
            .skip_frames(true)
            .scene_change_detect(true)
            .adaptive_quantization(false)
            .background_detection(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(0))
            .vui(VuiConfig::bt601());
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

    #[test]
    fn wrong_size_is_rejected() {
        let mut enc = H264Encoder::new(64, 64, 500_000).unwrap();
        let yuv = I420::new(32, 32);
        assert!(enc.encode(&yuv).is_err());
    }
}
