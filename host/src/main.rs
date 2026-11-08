use anyhow::Result;
use clap::Parser;
use penlink_host::protocol::DEFAULT_PORT;
use penlink_host::server::{self, Config};

/// Penlink host: streams one PC app full screen to the Penlink phone app.
#[derive(Parser, Debug)]
#[command(name = "penlink-host", version)]
struct Cli {
    /// Generate and show a new pairing code even if a phone is already paired.
    #[arg(long)]
    pair: bool,
    /// Also accept connections from private LAN addresses (default: loopback + Tailscale only).
    #[arg(long)]
    allow_lan: bool,
    /// TCP port to listen on.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// Token store file (default: %APPDATA%\Penlink\host.json). Used by tests and the stress tool.
    #[arg(long)]
    store: Option<std::path::PathBuf>,
    /// Do not use virtual monitors even if the SudoVDA driver is installed (window capture only).
    #[arg(long)]
    no_vdisplay: bool,
    /// Deliberately fault, to check that the crash report works.
    #[arg(long, hide = true)]
    selftest_crash: bool,
    /// Debug logging.
    #[arg(long, short)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let default = if cli.verbose { "penlink_host=debug,warn" } else { "penlink_host=info,warn" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| default.into()))
        .with_target(false)
        .init();

    penlink_host::crash::install(penlink_host::crash::default_dir());
    if cli.selftest_crash {
        unsafe { std::ptr::write_volatile(0x10 as *mut u32, 1) };
    }
    let cfg = Config {
        port: cli.port,
        allow_lan: cli.allow_lan,
        force_pair: cli.pair,
        store_path: cli.store,
        virtual_monitors: !cli.no_vdisplay,
        ..Config::default()
    };
    let mut srv = server::start(cfg).await?;
    println!(
        "Penlink host {} on port {}  ({}; {})",
        penlink_host::protocol::VERSION,
        srv.addr.port(),
        if cli.allow_lan { "loopback + Tailscale + LAN" } else { "loopback + Tailscale" },
        if srv.has_virtual_monitors() { "virtual monitors" } else { "window capture" }
    );
    // Ctrl+C, Ctrl+Break, closing the console window, logoff/shutdown: remove the virtual
    // monitors on the way out (the driver's watchdog would do it a few seconds later anyway).
    let mut ctrl_break = tokio::signal::windows::ctrl_break()?;
    let mut ctrl_close = tokio::signal::windows::ctrl_close()?;
    let mut ctrl_shutdown = tokio::signal::windows::ctrl_shutdown()?;
    let mut ctrl_logoff = tokio::signal::windows::ctrl_logoff()?;
    tokio::select! {
        _ = srv.wait() => {}
        _ = tokio::signal::ctrl_c() => { println!("shutting down"); }
        _ = ctrl_break.recv() => { println!("shutting down"); }
        _ = ctrl_close.recv() => { println!("shutting down"); }
        _ = ctrl_shutdown.recv() => { println!("shutting down"); }
        _ = ctrl_logoff.recv() => { println!("shutting down"); }
    }
    srv.shutdown().await;
    Ok(())
}
