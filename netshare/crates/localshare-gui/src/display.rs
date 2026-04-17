//! Query the primary display's real resolution and refresh rate.

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub width:  u32,
    pub height: u32,
    pub hz:     u32,
}

impl DisplayInfo {
    pub fn formatted(&self) -> String {
        format!("{} × {}", self.width, self.height)
    }
}

/// Best-effort query of the primary display. Returns `None` if the platform
/// query fails — callers should fall back to a placeholder.
pub fn primary() -> Option<DisplayInfo> {
    #[cfg(windows)]
    { primary_windows() }

    #[cfg(not(windows))]
    { None }
}

#[cfg(windows)]
fn primary_windows() -> Option<DisplayInfo> {
    use windows::Win32::Graphics::Gdi::{
        EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    };
    use windows::core::PCWSTR;

    unsafe {
        let mut dm: DEVMODEW = std::mem::zeroed();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;

        // PCWSTR::null() => the primary display
        let ok = EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut dm);
        if !ok.as_bool() {
            return None;
        }

        Some(DisplayInfo {
            width:  dm.dmPelsWidth,
            height: dm.dmPelsHeight,
            hz:     dm.dmDisplayFrequency,
        })
    }
}
