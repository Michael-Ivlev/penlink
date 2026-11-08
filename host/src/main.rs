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

    let cfg = Config { port: cli.port, allow_lan: cli.allow_lan, force_pair: cli.pair, ..Config::default() };
    let mut srv = server::start(cfg).await?;
    println!(
        "Penlink host {} on port {}  ({})",
        penlink_host::protocol::VERSION,
        srv.addr.port(),
        if cli.allow_lan { "loopback + Tailscale + LAN" } else { "loopback + Tailscale" }
    );
    tokio::select! {
        _ = srv.wait() => {}
        _ = tokio::signal::ctrl_c() => { println!("shutting down"); }
    }
    srv.shutdown().await;
    Ok(())
}
