//! NoExplorer backend.
//!
//! The webview UI (`/src`) calls the `#[tauri::command]` functions registered in [`run`]
//! via `invoke(...)`. Long transfers run as background jobs that stream progress to the UI
//! through the `"job"` event (see [`jobs`]) and to the tray menu (see [`tray`]).
//!
//! Module map (see `docs/ARCHITECTURE.md` for the full picture):
//! - [`config`]   — S3 accounts (profiles) + General settings: persisted JSON + S3 client
//! - [`state`]    — shared app state (settings, client, cancel flags)
//! - [`jobs`]     — background job bookkeeping + progress events
//! - [`remote`]   — S3 browsing: list, size, presign, read, mkdir, delete
//! - [`transfer`] — upload / download / open engine (multipart, concurrency)
//! - [`local`]    — local filesystem browsing for the left pane
//! - [`tray`]     — tray/menu-bar icon, live transfer status, background running
//! - [`autostart`] — launch at login (LaunchAgent with bundle id / Run key / .desktop)
//! - [`util`]     — small pure helpers (path safety, labels, error text)
//!
//! Plugins: opener, dialog, updater + process (auto-update & relaunch),
//! single-instance. Launch at login is our own [`autostart`] module (started with `--hidden`).

mod autostart;
mod box_api;
mod compare;
mod config;
mod fileops;
mod jobs;
mod local;
mod provider;
mod proxy;
mod remote;
mod resume;
mod s3;
mod scan;
mod secrets;
mod sizes;
mod state;
mod transfer;
mod trash;
mod tray;
mod util;
mod webdav;

use tauri::{Manager, WindowEvent};

/// Passed by the autostart launcher: start in the tray without showing the window.
pub(crate) const HIDDEN_ARG: &str = "--hidden";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Must be first: a second launch just focuses the running instance (which may be in the tray).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Previews of providers without presigned URLs (WebDAV) stream through nxr:// (proxy.rs).
        .register_asynchronous_uri_scheme_protocol("nxr", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move { responder.respond(proxy::handle(app, request).await) });
        })
        .manage(state::AppState::default())
        .setup(|app| {
            secrets::init(app.handle());
            let settings = config::load_settings(app.handle());
            resume::init(app.handle());
            let g = &settings.general;
            resume::set_speed_limit(if g.experimental.resume { g.speed_limit_kbps } else { 0 });
            app.state::<state::AppState>().set_settings(settings.clone());
            // Clean previews left over from earlier sessions.
            let _ = std::fs::remove_dir_all(transfer::temp_root());
            tray::setup(app.handle(), &settings)?;
            sizes::setup(app.handle());
            autostart::refresh(app.handle());
            if std::env::args().any(|a| a == HIDDEN_ARG) {
                tray::hide_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close button → hide to the tray (transfers keep running), unless disabled in Settings.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                if app.state::<state::AppState>().settings().general.run_in_background {
                    api.prevent_close();
                    tray::hide_main(app);
                }
            }
        })
        // Keep this list in sync with the command table in docs/ARCHITECTURE.md.
        .invoke_handler(tauri::generate_handler![
            config::get_settings,
            config::test_profile,
            config::save_profile,
            config::delete_profile,
            config::set_active_profile,
            config::save_general,
            autostart::autostart_enabled,
            autostart::set_autostart,
            remote::list_buckets,
            remote::list,
            remote::remote_size,
            remote::presign,
            remote::read_text,
            remote::mkdir,
            remote::delete,
            jobs::cancel,
            transfer::upload,
            transfer::download,
            transfer::open_object,
            local::list_local,
            local::local_size,
            local::volume_info,
            sizes::folder_sizes,
            webdav::nextcloud_login,
            webdav::cancel_login,
            box_api::box_login,
            compare::compare,
            transfer::upload_rel,
            transfer::download_rel,
            transfer::copy_remote,
            fileops::rename_local,
            fileops::move_local,
            fileops::duplicate_local,
            fileops::remote_move,
            resume::interrupted_jobs,
            resume::dismiss_interrupted,
            resume::resume_job,
            resume::pause,
            sizes::stop_sizes,
            trash::trash_local,
            local::read_local_text,
            local::mkdir_local,
            local::open_local,
            local::reveal,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        // macOS: clicking the Dock icon while the window is hidden brings it back.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            tray::show_main(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}
