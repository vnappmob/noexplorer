//! Browsing commands for the right pane: containers, list, size, link, read, mkdir, delete.
//! Thin wrappers over `provider::Remote` (ADR-025). Every command takes `account` (a profile
//! id); `None` means the active account.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry};
use crate::state::AppState;
use crate::util::{file_name, job_label};

/// Total size of a folder (or file / bucket root). Mirrors `SizeInfo` in `src/types.ts`.
#[derive(Serialize)]
pub struct SizeInfo {
    pub bytes: u64,
    /// Files only; folder marker objects are not counted.
    pub files: u64,
    /// Counting stopped at the cap: the numbers are a lower bound.
    pub partial: bool,
}

/// Containers (buckets) of an account.
#[tauri::command]
pub async fn list_buckets(state: State<'_, AppState>, account: Option<String>) -> Result<Vec<BucketInfo>, String> {
    state.remote(account.as_deref())?.containers().await
}

/// Recursive size of everything under `prefix` ("" = whole bucket). `cap` = automatic sizes
/// (experimental): count at most that many objects, S3 and Azure only.
#[tauri::command]
pub async fn remote_size(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    cap: Option<u64>,
) -> Result<SizeInfo, String> {
    let (bytes, files, partial) = state.remote(account.as_deref())?.folder_size(&bucket, &prefix, cap).await?;
    Ok(SizeInfo { bytes, files, partial })
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

/// Creates bucket / container `name` on `account`.
#[tauri::command]
pub async fn create_bucket(state: State<'_, AppState>, account: Option<String>, name: String) -> Result<(), String> {
    state.remote(account.as_deref())?.create_container(name.trim()).await
}

/// A bucket's access policy as pretty JSON, or None when it has none (experimental).
#[tauri::command]
pub async fn bucket_policy(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
) -> Result<Option<String>, String> {
    let p = state.remote(account.as_deref())?.policy(&bucket).await?;
    // Servers return it compact (AWS) or as stored: show it the same way everywhere.
    Ok(p.map(|p| {
        serde_json::from_str::<serde_json::Value>(&p)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or(p)
    }))
}

/// Sets (`policy` = JSON) or removes (`policy` = None) a bucket's access policy (experimental).
#[tauri::command]
pub async fn set_bucket_policy(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    policy: Option<String>,
) -> Result<(), String> {
    let policy = policy.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = &policy {
        serde_json::from_str::<serde_json::Value>(p).map_err(|e| format!("The policy isn't valid JSON: {e}"))?;
    }
    state.remote(account.as_deref())?.set_policy(&bucket, policy.as_deref()).await
}

/// The public (unsigned) address of `key`, for files a policy makes public (experimental).
#[tauri::command]
pub async fn public_url(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    key: String,
) -> Result<String, String> {
    state.remote(account.as_deref())?.public_url(&bucket, &key).await
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
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    keys: Vec<String>,
) -> Result<String, String> {
    // A background job like transfers (kind "delete"): a big folder can take a while, and the UI
    // shows its progress and lets it be cancelled. `prefix` = the folder the keys are in.
    let r = state.remote(account.as_deref())?;
    let first = keys.first().ok_or("Nothing to delete")?;
    let prefix = match first.trim_end_matches('/').rfind('/') {
        Some(i) => first[..=i].to_string(),
        None => String::new(),
    };
    let label = job_label(keys.iter().map(|k| file_name(k.trim_end_matches('/'))).collect());
    let job = Job::new(&app, "delete", label, r.account(), &bucket, &prefix);
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        if let Err(e) = r.delete(&bucket, keys, Some(&j)).await
            && e != "cancelled"
        {
            j.fail("Delete", e);
        }
    });
    Ok(id)
}
