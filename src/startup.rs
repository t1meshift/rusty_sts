const APP_NAME: &str = "rusty-sts";

#[cfg(windows)]
mod imp {
    use super::APP_NAME;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn enable_autostart() -> Result<(), String> {
        let exe_path =
            std::env::current_exe().map_err(|e| format!("Failed to get exe path: {e}"))?;
        let value = format!("\"{}\" --minimized", exe_path.display());

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run_key, _) = hkcu
            .create_subkey(RUN_KEY)
            .map_err(|e| format!("Failed to open registry key: {e}"))?;
        run_key
            .set_value(APP_NAME, &value)
            .map_err(|e| format!("Failed to write registry value: {e}"))?;
        Ok(())
    }

    pub fn disable_autostart() -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run_key = hkcu
            .open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_WRITE)
            .map_err(|e| format!("Failed to open registry key: {e}"))?;
        let _ = run_key.delete_value(APP_NAME);
        Ok(())
    }

    pub fn is_autostart_registered() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.open_subkey(RUN_KEY)
            .and_then(|k| k.get_value::<String, _>(APP_NAME))
            .is_ok()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::APP_NAME;
    use std::path::PathBuf;

    fn desktop_file_path() -> Result<PathBuf, String> {
        dirs::config_dir()
            .map(|p| p.join("autostart").join(format!("{APP_NAME}.desktop")))
            .ok_or_else(|| "Could not determine config directory".to_string())
    }

    pub fn enable_autostart() -> Result<(), String> {
        let exe_path =
            std::env::current_exe().map_err(|e| format!("Failed to get exe path: {e}"))?;
        let path = desktop_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create autostart directory: {e}"))?;
        }
        let contents = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name={APP_NAME}\n\
             Comment=Sync Slay the Spire 2 runs to the stats tracker\n\
             Exec=\"{}\" --minimized\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n",
            exe_path.display()
        );
        std::fs::write(&path, contents)
            .map_err(|e| format!("Failed to write autostart entry: {e}"))?;
        Ok(())
    }

    pub fn disable_autostart() -> Result<(), String> {
        let path = desktop_file_path()?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Failed to remove autostart entry: {e}")),
        }
    }

    pub fn is_autostart_registered() -> bool {
        desktop_file_path().map(|p| p.is_file()).unwrap_or(false)
    }
}

pub use imp::{disable_autostart, enable_autostart};

/// If autostart is registered, rewrite the entry so it points at the
/// current executable path (the binary may have moved since it was set).
pub fn refresh_autostart_if_needed() {
    if imp::is_autostart_registered() {
        let _ = imp::enable_autostart();
    }
}
