//! Read-only: prints a line (Unix time in ms) whenever the set of monitors changes.
//!
//!     monwatch [--secs 60]
//!
//! Used to time how fast a virtual monitor disappears after the phone closes a session.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() {
    penlink_host::winutil::init_process();
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.iter().position(|a| a == "--secs").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(60);
    let end = Instant::now() + Duration::from_secs(secs);
    let mut last: Option<Vec<String>> = None;
    while Instant::now() < end {
        let mut now: Vec<String> = penlink_host::vdisplay::enum_monitors().iter().map(|m| format!("{} {}x{}@{},{}", m.name, m.rect.w, m.rect.h, m.rect.x, m.rect.y)).collect();
        now.sort();
        if last.as_ref() != Some(&now) {
            let ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
            println!("{ms} monitors={} [{}]", now.len(), now.join(" | "));
            last = Some(now);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
