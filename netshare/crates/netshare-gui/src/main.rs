mod app;
mod discovery;
mod tray;

use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    // GTK must be initialised before tray-icon creates any menus (Linux only).
    #[cfg(target_os = "linux")]
    gtk::init().expect("Failed to initialise GTK");

    // Install rustls crypto provider (ring) as the process-level default.
    // Must be called before any TLS code runs.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("netshare=debug".parse()?))
        .init();

    // Start a multi-thread tokio runtime; eframe occupies the main thread.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let handle = rt.handle().clone();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NetShare")
            .with_inner_size([480.0, 520.0])
            .with_min_inner_size([380.0, 400.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "NetShare",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::NetShareApp::new(cc, handle)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}
