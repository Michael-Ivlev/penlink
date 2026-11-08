//! Looks at the SudoVDA virtual display driver and, on request, makes one Penlink monitor.
//!
//!     vdcheck                      driver present? protocol, watchdog, GPUs, current monitors
//!     vdcheck --make 1440x3120 --dpi 300 --secs 8
//!                                  also create a monitor, show where it is, remove it, and
//!                                  prove that the real monitors did not change
//!
//! Only monitors this tool creates are touched (GUIDs in the Penlink namespace, listed in
//! %APPDATA%\Penlink\vdisplays.json).

use penlink_host::vdisplay;
use std::time::Duration;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    penlink_host::winutil::init_process();
    let args: Vec<String> = std::env::args().collect();

    println!("SudoVDA installed: {}", vdisplay::is_installed());
    for (a, luid) in vdisplay::list_adapters() {
        println!(
            "  GPU {:?} vendor {:#06x} vram {} MB software={} luid {:08x}:{:08x}",
            a.description,
            a.vendor_id,
            a.dedicated_vram / (1 << 20),
            a.software,
            luid.HighPart,
            luid.LowPart
        );
    }
    let before = vdisplay::monitor_snapshot();
    println!("monitors now:");
    for l in &before {
        println!("  {l}");
    }

    let Some(make) = arg(&args, "--make") else {
        if !vdisplay::is_installed() {
            println!("The driver is not installed: Penlink falls back to window capture.");
        }
        return;
    };
    let (w, h) = make
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("--make WxH, e.g. 1440x3120");
    let dpi: u32 = arg(&args, "--dpi").and_then(|v| v.parse().ok()).unwrap_or(300);
    let secs: u64 = arg(&args, "--secs").and_then(|v| v.parse().ok()).unwrap_or(5);

    let Some(mgr) = vdisplay::start_manager() else {
        println!("cannot open the driver");
        std::process::exit(2);
    };
    match mgr.create(w, h, dpi) {
        Ok(m) => {
            println!(
                "created {} at {},{} size {}x{} dpi wanted {}% actual {}%",
                m.gdi_name, m.rect.x, m.rect.y, m.rect.w, m.rect.h, m.dpi_wanted, m.dpi_actual
            );
            if let Some(p) = vdisplay::dpi_percent(&m.gdi_name) {
                println!("  Windows reports DPI {p}%");
            }
            println!("monitors with the virtual one:");
            for l in vdisplay::monitor_snapshot() {
                println!("  {l}");
            }
            std::thread::sleep(Duration::from_secs(secs));
            mgr.release(&m.guid);
        }
        Err(e) => println!("create failed: {e:#}"),
    }
    std::thread::sleep(Duration::from_millis(500));
    let after = vdisplay::monitor_snapshot();
    println!("monitors after removal:");
    for l in &after {
        println!("  {l}");
    }
    println!("{}", if before == after { "REAL MONITORS UNCHANGED" } else { "MONITORS DIFFER (see above)" });
}
