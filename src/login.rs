//! "Open at login": starts Scotshreen when the user logs in.
//!
//! - macOS: a LaunchAgent in `~/Library/LaunchAgents` that runs this executable.
//! - Windows: a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! Both point at the executable that is running now, so moving the app means turning this off
//! and on again.

/// True if this platform supports the setting (the tray only shows it then).
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[cfg(target_os = "macos")]
const LABEL: &str = "app.scotshreen.login";

#[cfg(target_os = "macos")]
fn plist_path() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

/// Whether Scotshreen starts at login.
#[cfg(target_os = "macos")]
pub fn is_enabled() -> bool {
    plist_path().is_some_and(|p| p.exists())
}

/// Turns starting at login on or off.
#[cfg(target_os = "macos")]
pub fn set_enabled(on: bool) -> Result<(), String> {
    let path = plist_path().ok_or("no home folder")?;
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = xml_escape(&exe.to_string_lossy());
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, plist).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(target_os = "windows")]
mod win {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Scotshreen";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn is_enabled() -> bool {
        let (key, name) = (wide(KEY), wide(NAME));
        // SAFETY: valid, NUL-terminated strings; no output buffer is asked for.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS
    }

    pub fn set_enabled(on: bool) -> Result<(), String> {
        let (key, name) = (wide(KEY), wide(NAME));
        let status = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let value = wide(&format!("\"{}\"", exe.display()));
            // SAFETY: valid, NUL-terminated strings; the size is the value's length in bytes.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                )
            }
        } else {
            // SAFETY: valid, NUL-terminated strings.
            let s = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
            // Already gone counts as done.
            if s == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND { ERROR_SUCCESS } else { s }
        };
        if status == ERROR_SUCCESS { Ok(()) } else { Err(format!("registry error {status}")) }
    }
}

#[cfg(target_os = "windows")]
pub use win::{is_enabled, set_enabled};

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn set_enabled(_on: bool) -> Result<(), String> {
    Err("not supported on this platform".into())
}
