use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    let _handle = netshare_client::start(&server_addr_str, &client_name)?;
    info!("All subsystems started — press Ctrl+C to stop");

    tokio::signal::ctrl_c().await?;
    info!("Shutting down");
    Ok(())
}
