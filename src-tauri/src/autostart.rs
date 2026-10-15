//! Launch at login (Settings → General). Replaces tauri-plugin-autostart so we can set the
//! LaunchAgent's `AssociatedBundleIdentifiers` on macOS. Without it, System Settings → Login
//! Items lists the background item under the *code-signing developer's name* (e.g. "Khoa Luu")
//! instead of "NoExplorer". See ADR-018.
//!
//! The entry starts the app with `--hidden`, so it comes up in the tray (see `lib.rs`).

use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use tauri::AppHandle;

use crate::HIDDEN_ARG;

fn launcher(app: &AppHandle) -> Result<AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut b = AutoLaunchBuilder::new();
    // Same label as the old plugin used, so enabling overwrites a previous entry in place.
    b.set_app_name(&app.package_info().name).set_args(&[HIDDEN_ARG]);
    #[cfg(target_os = "macos")]
    {
        let exe = exe.canonicalize().map_err(|e| e.to_string())?;
        b.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
            .set_bundle_identifiers(&[app.config().identifier.as_str()])
            .set_app_path(&exe.to_string_lossy());
    }
    #[cfg(target_os = "windows")]
    b.set_app_path(&exe.to_string_lossy());
    #[cfg(target_os = "linux")]
    {
        // Point at the AppImage itself (not the temporary mount) when running from one.
        let path = std::env::var("APPIMAGE").unwrap_or_else(|_| exe.to_string_lossy().into_owned());
        b.set_app_path(&path);
    }
    b.build().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn autostart_enabled(app: AppHandle) -> Result<bool, String> {
    launcher(&app)?.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let l = launcher(&app)?;
    if enabled { l.enable() } else { l.disable() }.map_err(|e| e.to_string())
}

/// Rewrites an existing entry at startup: fixes entries created by older versions (no bundle
/// identifier → shown under the developer's name) and follows the app if it was moved.
pub fn refresh(app: &AppHandle) {
    if let Ok(l) = launcher(app)
        && l.is_enabled().unwrap_or(false)
    {
        let _ = l.enable();
    }
}
