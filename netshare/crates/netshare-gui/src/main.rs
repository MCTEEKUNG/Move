mod app;
mod discovery;
mod tray;

// ── Windows elevation helpers (used by the Settings page) ────────────────────

/// Returns true when the current process has Administrator / HIGH integrity.
#[cfg(target_os = "windows")]
pub(crate) fn windows_is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return true; // assume elevated to avoid false warnings
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn windows_is_elevated() -> bool { true }

/// Re-launches the current executable with the "runas" verb (UAC prompt).
/// Returns true when the elevated process was successfully spawned.
#[cfg(target_os = "windows")]
pub(crate) fn windows_relaunch_elevated() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0u16)).collect()
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe_w  = to_wide(exe.to_str().unwrap_or(""));
    let verb_w = to_wide("runas");
    // Pass a marker so the relaunched process can detect it is already elevated.
    let args_w = to_wide("--already-elevated");
    unsafe {
        let h = ShellExecuteW(
            None,
            PCWSTR(verb_w.as_ptr()),
            PCWSTR(exe_w.as_ptr()),
            PCWSTR(args_w.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        h.0 as isize > 32
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn windows_relaunch_elevated() -> bool { false }

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
