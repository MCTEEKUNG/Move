use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("netshare=debug".parse()?))
        .init();

    let bind_addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0.0.0.0:9000".to_owned());

    info!("NetShare Server — binding on {bind_addr}");

    let _handle = netshare_server::start(&bind_addr)?;
    info!("All subsystems started — press Ctrl+C to stop");

    tokio::signal::ctrl_c().await?;
    info!("Shutting down");
    Ok(())
}
