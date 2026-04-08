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

    let server_addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9000".to_owned());

    let client_name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "client".to_owned());

    info!("NetShare Client → {server_addr_str} as '{client_name}'");

    let pairing_code = std::env::args().nth(2).unwrap_or_default();
    let _handle = netshare_client::start(&server_addr_str, &client_name, &pairing_code)?;
    info!("All subsystems started — press Ctrl+C to stop");

    tokio::signal::ctrl_c().await?;
    info!("Shutting down");
    Ok(())
}
