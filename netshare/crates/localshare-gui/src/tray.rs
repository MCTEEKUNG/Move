//! System tray icon management.
//!
//! Shows a small icon in the system tray.  Left-click → show/hide window.
//! Right-click menu: Show, ─, Quit.

use tray_icon::{
    TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, MenuId, PredefinedMenuItem},
};

pub struct AppTray {
    _icon: TrayIcon, // kept alive
    quit_id: MenuId,
    _show_id: MenuId,
}

impl AppTray {
    pub fn new() -> anyhow::Result<Self> {
        let menu = Menu::new();
        let show_item = MenuItem::new("Open LocalShare", true, None);
        let quit_item = MenuItem::new("Quit", true, None);

        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();

        menu.append(&show_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        // Small 16×16 RGBA icon — steel-blue square as a placeholder
        let icon = make_icon();

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("LocalShare")
            .with_icon(icon)
            .build()?;

        Ok(Self { _icon: tray, quit_id, _show_id: show_id })
    }

    /// Call once per frame to drain pending tray events.
    /// Returns a `TrayPoll` describing what happened.
    pub fn poll(&self) -> TrayPoll {
        let mut result = TrayPoll::default();

        // Menu events
        while let Ok(evt) = MenuEvent::receiver().try_recv() {
            if evt.id == self.quit_id {
                result.quit = true;
            }
        }

        // Tray icon click events
        while let Ok(evt) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { .. } = evt {
                result.toggle_window = true;
            }
        }

        result
    }
}

#[derive(Default)]
pub struct TrayPoll {
    pub quit:          bool,
    pub toggle_window: bool,
}

fn make_icon() -> tray_icon::Icon {
    // 16×16 steel-blue square as the tray icon
    const SIZE: usize = 16;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for i in 0..SIZE * SIZE {
        let base = i * 4;
        rgba[base]     = 82;   // R
        rgba[base + 1] = 130;  // G
        rgba[base + 2] = 195;  // B
        rgba[base + 3] = 255;  // A
    }
    tray_icon::Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)
        .expect("icon")
}
