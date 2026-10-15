//! NoExplorer backend.
//!
//! The webview UI (`/src`) calls the `#[tauri::command]` functions registered in [`run`]
//! via `invoke(...)`. Long transfers run as background jobs that stream progress to the UI
//! through the `"job"` event (see [`jobs`]).
//!
//! Module map (see `docs/ARCHITECTURE.md` for the full picture):
//! - [`config`]   — connection settings: persisted JSON + S3 client construction
//! - [`state`]    — shared app state (config, client, cancel flags)
//! - [`jobs`]     — background job bookkeeping + progress events
//! - [`remote`]   — S3 browsing: list, presign, read, mkdir, delete
//! - [`transfer`] — upload / download / open engine (multipart, concurrency)
//! - [`local`]    — local filesystem browsing for the left pane
//! - [`util`]     — small pure helpers (path safety, labels, error text)

mod config;
mod jobs;
mod local;
mod remote;
mod state;
mod transfer;
mod util;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .setup(|app| {
            if let Some(cfg) = config::load_config(app.handle()) {
                app.state::<state::AppState>().set_config(cfg);
            }
            // Clean previews left over from earlier sessions.
            let _ = std::fs::remove_dir_all(transfer::temp_root());
            Ok(())
        })
        // Keep this list in sync with the command table in docs/ARCHITECTURE.md.
        .invoke_handler(tauri::generate_handler![
            config::get_config,
            config::test_config,
            config::save_config,
            config::list_buckets,
            remote::list,
            remote::presign,
            remote::read_text,
            remote::mkdir,
            remote::delete,
            jobs::cancel,
            transfer::upload,
            transfer::download,
            transfer::open_object,
            local::list_local,
            local::read_local_text,
            local::mkdir_local,
            local::open_local,
            local::reveal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
