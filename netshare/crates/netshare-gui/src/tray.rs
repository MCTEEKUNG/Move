/// System tray icon management.
///
/// Uses the `tray-icon` crate which supports Windows and Linux (AppIndicator).
/// The tray icon must be created on the main thread (message-loop thread).
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

pub struct TrayHandle {
    pub _tray: TrayIcon,
    pub show_hide_id: tray_icon::menu::MenuId,
    pub quit_id:      tray_icon::menu::MenuId,
}

impl TrayHandle {
    pub fn create() -> anyhow::Result<Self> {
        let icon = make_icon();

        let show_hide = MenuItem::new("Show / Hide", true, None);
        let quit      = MenuItem::new("Quit", true, None);
        let show_hide_id = show_hide.id().clone();
        let quit_id      = quit.id().clone();

        let menu = Menu::new();
        menu.append(&show_hide)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("NetShare")
            .with_menu(Box::new(menu))
            .build()?;

        Ok(Self { _tray: tray, show_hide_id, quit_id })
    }

    /// Update the tooltip text (e.g. show active client name).
    pub fn set_tooltip(&self, text: &str) {
        let _ = self._tray.set_tooltip(Some(text.to_owned()));
    }

    /// Drain tray click events; returns `true` if the window should toggle visibility.
    pub fn poll_toggle(&self) -> bool {
        let mut toggle = false;
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { .. } = event {
                toggle = true;
            }
        }
        toggle
    }

    /// Drain menu events; returns `true` if Quit was selected.
    pub fn poll_quit(&self) -> bool {
        let mut quit = false;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.quit_id {
                quit = true;
            } else if event.id == self.show_hide_id {
                // toggle is handled via poll_toggle for clicks; menu also works
            }
        }
        quit
    }
}

/// Build a simple 32×32 RGBA icon: dark-blue background with a white "N".
fn make_icon() -> Icon {
    const W: usize = 32;
    const H: usize = 32;
    let mut data = vec![0u8; W * H * 4];

    // Fill background (#1E3A5F — dark blue).
    for i in (0..data.len()).step_by(4) {
        data[i]     = 0x1E; // R
        data[i + 1] = 0x3A; // G
        data[i + 2] = 0x5F; // B
        data[i + 3] = 0xFF; // A
    }

    // Draw a simple "N" glyph in white, pixel-art style (7×11 at offset 12,10).
    const GLYPH: &[(usize, usize)] = &[
        (0,0),(0,1),(0,2),(0,3),(0,4),(0,5),(0,6),(0,7),(0,8),(0,9),(0,10),
        (1,1),(2,2),(3,3),(4,4),(5,5),(6,6),
        (6,0),(6,1),(6,2),(6,3),(6,4),(6,5),(6,6),(6,7),(6,8),(6,9),(6,10),
    ];
    for &(gx, gy) in GLYPH {
        let px = (gx + 12).min(W - 1);
        let py = (gy + 10).min(H - 1);
        let idx = (py * W + px) * 4;
        data[idx]     = 0xFF;
        data[idx + 1] = 0xFF;
        data[idx + 2] = 0xFF;
        data[idx + 3] = 0xFF;
    }

    Icon::from_rgba(data, W as u32, H as u32).expect("icon creation failed")
}
