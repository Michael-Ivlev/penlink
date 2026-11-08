//! Latency instrumentation (diagnostics only, near-free when idle).
//!
//! Splits "phone input -> visible result" on the host into stages. All times are microseconds on
//! one monotonic clock ([`now_us`]). The phone gets the same numbers as `lat` messages on the
//! `input` data channel and shows them in its debug overlay (`ping`/`pong` messages let it
//! estimate the data channel round trip, and so the one-way network time).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static EPOCH: OnceLock<Instant> = OnceLock::new();

pub fn now_us() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// The newest input event and whether a captured frame has been matched to it yet.
pub struct Probe {
    /// When the data channel message arrived at the host.
    pub recv_us: AtomicU64,
    /// When it had been injected (SendInput returned).
    pub inject_us: AtomicU64,
    /// Number of injected input events; a record is produced once per event.
    pub seq: AtomicU64,
    pub reported: AtomicU64,
}

pub static PROBE: Probe = Probe { recv_us: AtomicU64::new(0), inject_us: AtomicU64::new(0), seq: AtomicU64::new(0), reported: AtomicU64::new(0) };

impl Probe {
    pub fn injected(&self, recv_us: u64) {
        self.recv_us.store(recv_us, Ordering::SeqCst);
        self.inject_us.store(now_us(), Ordering::SeqCst);
        self.seq.fetch_add(1, Ordering::SeqCst);
    }

    /// Called by the capture pipeline once a frame is on the wire. Returns a record when this is
    /// the first frame captured after the newest injected input.
    pub fn frame_sent(&self, st: FrameStages) -> Option<LatRecord> {
        let seq = self.seq.load(Ordering::SeqCst);
        if seq == self.reported.load(Ordering::SeqCst) {
            return None;
        }
        let inj = self.inject_us.load(Ordering::SeqCst);
        if st.arrived_us < inj {
            return None; // frame was already captured when the input was injected
        }
        self.reported.store(seq, Ordering::SeqCst);
        let recv = self.recv_us.load(Ordering::SeqCst);
        Some(LatRecord {
            recv_to_inject: inj.saturating_sub(recv),
            inject_to_capture: st.arrived_us - inj,
            capture_to_encode_start: st.encode_start_us.saturating_sub(st.arrived_us),
            encode: st.encode_end_us.saturating_sub(st.encode_start_us),
            send: st.sent_us.saturating_sub(st.encode_end_us),
            host_us: st.sent_us,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStages {
    /// WGC `FrameArrived` for the frame that was encoded.
    pub arrived_us: u64,
    pub encode_start_us: u64,
    pub encode_end_us: u64,
    pub sent_us: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct LatRecord {
    pub recv_to_inject: u64,
    pub inject_to_capture: u64,
    pub capture_to_encode_start: u64,
    pub encode: u64,
    pub send: u64,
    pub host_us: u64,
}

impl LatRecord {
    pub fn total_us(&self) -> u64 {
        self.recv_to_inject + self.inject_to_capture + self.capture_to_encode_start + self.encode + self.send
    }

    pub fn json(&self) -> String {
        format!(
            "{{\"t\":\"lat\",\"inj\":{},\"cap\":{},\"conv\":{},\"enc\":{},\"snd\":{},\"h\":{}}}",
            self.recv_to_inject, self.inject_to_capture, self.capture_to_encode_start, self.encode, self.send, self.host_us
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_only_for_first_frame_after_input() {
        let p = Probe { recv_us: AtomicU64::new(0), inject_us: AtomicU64::new(0), seq: AtomicU64::new(0), reported: AtomicU64::new(0) };
        let st = FrameStages { arrived_us: 100, encode_start_us: 110, encode_end_us: 130, sent_us: 135 };
        assert!(p.frame_sent(st).is_none(), "no input yet");
        p.recv_us.store(40, Ordering::SeqCst);
        p.inject_us.store(50, Ordering::SeqCst);
        p.seq.store(1, Ordering::SeqCst);
        let r = p.frame_sent(st).expect("record");
        assert_eq!((r.recv_to_inject, r.inject_to_capture, r.capture_to_encode_start, r.encode, r.send), (10, 50, 10, 20, 5));
        assert_eq!(r.total_us(), 95);
        assert!(p.frame_sent(st).is_none(), "only once per input");
        p.inject_us.store(200, Ordering::SeqCst);
        p.seq.store(2, Ordering::SeqCst);
        assert!(p.frame_sent(st).is_none(), "frame older than the input");
    }
}
