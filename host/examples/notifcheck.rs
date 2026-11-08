//! Check of the Windows notification listener from this (unpackaged) exe.
//!
//!     notifcheck          GetAccessStatus only (never prompts, reads nothing)
//!     notifcheck --list   also GetNotificationsAsync(Toast) and prints app name + id + text lengths
//!                         (not the text) of what is in the Action Center. Never calls RequestAccessAsync.

use penlink_host::notifications::{Source, WinSource};

fn main() {
    penlink_host::winutil::init_process();
    let src = WinSource::new();
    println!("access status: {:?}", src.access());
    if std::env::args().any(|a| a == "--list") {
        match src.fetch() {
            Ok(v) => {
                println!("{} notification(s) readable", v.len());
                for n in v {
                    println!(
                        "  id {} app {:?} ({}) title {} chars, body {} chars, logo {}",
                        n.id,
                        n.app_name,
                        n.app_id,
                        n.title.chars().count(),
                        n.body.chars().count(),
                        src.logo(&n.app_id).map(|l| format!("{} bytes", l.len())).unwrap_or_else(|| "none".into())
                    );
                }
            }
            Err(e) => println!("GetNotificationsAsync failed: {e}"),
        }
    }
}
