//! Register LocalShare daemon to start automatically at login.
//!
//! Windows: HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//! Linux:   ~/.config/systemd/user/localshare.service

/// Register this binary for auto-start. Idempotent — safe to call every run.
pub fn register() -> anyhow::Result<()> {
    #[cfg(windows)]
    windows_register()?;

    #[cfg(unix)]
    linux_register()?;

    Ok(())
}

#[cfg(windows)]
fn windows_register() -> anyhow::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::System::Registry::{
        RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_SZ,
    };

    let exe = std::env::current_exe()?;
    let exe_str = format!("\"{}\"", exe.display());

    let sub_key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16().collect();
    let value_name: Vec<u16> = "LocalShare\0".encode_utf16().collect();
    let value_data: Vec<u16> = exe_str.encode_utf16().chain([0u16]).collect();

    unsafe {
        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        let err = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub_key.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        );
        if err != WIN32_ERROR(0) {
            anyhow::bail!("RegOpenKeyExW failed: {:?}", err);
        }

        let err = RegSetValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            0,
            REG_SZ,
            Some(std::slice::from_raw_parts(
                value_data.as_ptr() as *const u8,
                value_data.len() * 2,
            )),
        );
        if err != WIN32_ERROR(0) {
            anyhow::bail!("RegSetValueExW failed: {:?}", err);
        }
    }

    tracing::info!("Auto-start registered in Windows registry");
    Ok(())
}

#[cfg(unix)]
fn linux_register() -> anyhow::Result<()> {
    let exe     = std::env::current_exe()?;
    let home    = std::env::var("HOME")?;
    let svc_dir = format!("{home}/.config/systemd/user");
    std::fs::create_dir_all(&svc_dir)?;

    let unit = format!(
        "[Unit]\n\
         Description=LocalShare Daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe}\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = exe.display()
    );

    let path = format!("{svc_dir}/localshare.service");
    std::fs::write(&path, unit)?;
    tracing::info!("Systemd user service written to {path}");

    // Enable the service (non-fatal if systemctl isn't available)
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "enable", "localshare.service"])
        .status();

    Ok(())
}
