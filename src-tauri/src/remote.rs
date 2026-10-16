//! Browsing commands for the right pane: containers, list, size, link, read, mkdir, delete.
//! Thin wrappers over `provider::Remote` (ADR-025). Every command takes `account` (a profile
//! id); `None` means the active account.

use serde::Serialize;
use tauri::State;

use crate::provider::{BucketInfo, Entry};
use crate::state::AppState;

/// Total size of a folder (or file / bucket root). Mirrors `SizeInfo` in `src/types.ts`.
#[derive(Serialize)]
pub struct SizeInfo {
    pub bytes: u64,
    /// Files only; folder marker objects are not counted.
    pub files: u64,
}

/// Containers (buckets) of an account.
#[tauri::command]
pub async fn list_buckets(state: State<'_, AppState>, account: Option<String>) -> Result<Vec<BucketInfo>, String> {
    state.remote(account.as_deref())?.containers().await
}

/// Recursive size of everything under `prefix` ("" = whole bucket).
#[tauri::command]
pub async fn remote_size(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
) -> Result<SizeInfo, String> {
    let all = state.remote(account.as_deref())?.list_all(&bucket, &prefix).await?;
    let files = all.iter().filter(|(k, _)| !k.ends_with('/'));
    Ok(SizeInfo { bytes: files.clone().map(|(_, s)| s).sum(), files: files.count() as u64 })
}

/// One folder level: sub-folders and files directly under `prefix`.
#[tauri::command]
pub async fn list(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
) -> Result<Vec<Entry>, String> {
    state.remote(account.as_deref())?.list(&bucket, &prefix).await
}

/// Time-limited GET URL. `inline` makes browsers render it (preview) instead of downloading.
#[tauri::command]
pub async fn presign(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    key: String,
    secs: u64,
    inline: bool,
) -> Result<String, String> {
    state.remote(account.as_deref())?.link(&bucket, &key, secs, inline).await
}

/// First `max` bytes of a file as (lossy) UTF-8, for the text preview.
#[tauri::command]
pub async fn read_text(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    key: String,
    max: u64,
) -> Result<String, String> {
    let bytes = state.remote(account.as_deref())?.read_head(&bucket, &key, max).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Creates folder `name` under `prefix`.
#[tauri::command]
pub async fn mkdir(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    name: String,
) -> Result<(), String> {
    let name = name.trim().trim_matches('/');
    if name.is_empty() {
        return Err("Folder name is empty".into());
    }
    state.remote(account.as_deref())?.mkdir(&bucket, &prefix, name).await
}

/// Deletes files and folders (keys ending in `/`, recursively). Returns the count.
#[tauri::command]
pub async fn delete(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    keys: Vec<String>,
) -> Result<usize, String> {
    state.remote(account.as_deref())?.delete(&bucket, keys).await
}
