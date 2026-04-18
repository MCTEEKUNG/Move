mod app;
mod bridge;
mod display;
mod edge;
mod theme;
mod tray;
mod views;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    // Tray icon MUST be created on the main thread before the event loop.
    let _tray = tray::AppTray::new().ok(); // non-fatal if tray unavailable

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LocalShare")
            .with_inner_size([440.0, 580.0])
            .with_min_inner_size([380.0, 440.0])
            .with_resizable(true),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "LocalShare",
        options,
        Box::new(|cc| Ok(Box::new(app::LocalShareApp::new(cc)))),
    )
}
