//! Launch at login (Settings → General). Replaces tauri-plugin-autostart so we can set the
//! LaunchAgent's `AssociatedBundleIdentifiers` on macOS. Without it, System Settings → Login
//! Items lists the background item under the *code-signing developer's name* (e.g. "Khoa Luu")
//! instead of "NoExplorer". See ADR-018.
//!
//! The entry starts the app with `--hidden`, so it comes up in the tray (see `lib.rs`).
//!
//! macOS Background Task Management decides an item's parent (app vs. developer) only when
//! the item is first *added*; rewriting the plist later doesn't re-parent it. So entries
//! created before the fix are removed and re-added once (see [`refresh`], [`ENTRY_VERSION`]).

use std::path::PathBuf;
use std::time::Duration;

use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use tauri::{AppHandle, Manager};

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

/// Bump when the OS entry must be re-created from scratch (not just rewritten).
/// 2 = macOS LaunchAgent with AssociatedBundleIdentifiers, registered fresh (v0.2.3).
const ENTRY_VERSION: &str = "2";
/// How long macOS needs to notice the plist is gone before we add it back.
const READD_DELAY: Duration = Duration::from_secs(12);

fn marker(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("autostart-entry-version"))
}

fn mark_current(app: &AppHandle) {
    if let Some(m) = marker(app) {
        let _ = std::fs::create_dir_all(m.parent().unwrap());
        let _ = std::fs::write(m, ENTRY_VERSION);
    }
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let l = launcher(&app)?;
    if enabled { l.enable() } else { l.disable() }.map_err(|e| e.to_string())?;
    if enabled {
        mark_current(&app); // a fresh registration is already the current kind
    }
    Ok(())
}

/// At startup, if launch-at-login is on: re-create entries from older versions once
/// (remove → wait for macOS to drop it → add), otherwise just rewrite it so it follows a
/// moved app. Runs on a background thread; the app doesn't wait for it.
pub fn refresh(app: &AppHandle) {
    let Ok(l) = launcher(app) else { return };
    if !l.is_enabled().unwrap_or(false) {
        return;
    }
    let stale = marker(app).and_then(|m| std::fs::read_to_string(m).ok()).as_deref() != Some(ENTRY_VERSION);
    if !(cfg!(target_os = "macos") && stale) {
        let _ = l.enable();
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if l.disable().is_err() {
            return;
        }
        std::thread::sleep(READD_DELAY);
        if l.enable().is_ok() {
            mark_current(&app);
        }
    });
}
