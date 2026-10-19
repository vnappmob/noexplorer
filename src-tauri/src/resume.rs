//! Resumable transfers: the global speed limit, retries, and two small
//! journals in the app config folder that survive a quit or crash.
//!
//! - `pending.json`: transfers that started but didn't finish. On the next launch the UI offers to
//!   resume them; a resumed upload skips files already on the remote with the same size, a resumed
//!   download skips files already complete and continues `.part` files with a ranged GET.
//! - `multipart.json`: S3 multipart uploads in progress (upload id per file). A resumed upload of
//!   the same unchanged file lists the parts already on the server and only sends the rest.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;
use crate::transfer::DlItem;

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// Serializes journal read-modify-write cycles (many files finish at once).
static LOCK: Mutex<()> = Mutex::new(());

/// Tests: journals in a temp folder instead of the app config folder.
#[cfg(test)]
pub fn init_dir(d: PathBuf) {
    let _ = DIR.set(d);
}

pub fn init(app: &AppHandle) {
    if let Ok(d) = app.path().app_config_dir() {
        let _ = DIR.set(d);
    }
}

fn load<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    DIR.get()
        .and_then(|d| std::fs::read(d.join(name)).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(name: &str, v: &T) {
    if let Some(d) = DIR.get() {
        let _ = std::fs::create_dir_all(d);
        if let Ok(b) = serde_json::to_vec_pretty(v) {
            let _ = std::fs::write(d.join(name), b);
        }
    }
}

// ---------- speed limit ----------

/// Bytes per second for all transfers together; 0 = no limit.
static LIMIT: AtomicU64 = AtomicU64::new(0);
/// When the shared budget is next free (a token bucket over every running transfer).
static NEXT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn set_speed_limit(kbps: u64) {
    LIMIT.store(kbps * 1024, Ordering::Relaxed);
}

/// Waits long enough that `n` more bytes keep everyone under the limit. Called after each chunk.
pub async fn throttle(n: u64) {
    let limit = LIMIT.load(Ordering::Relaxed);
    if limit == 0 || n == 0 {
        return;
    }
    let wait = {
        let mut next = NEXT.lock().unwrap();
        let now = Instant::now();
        let start = next.filter(|t| *t > now).unwrap_or(now);
        let end = start + Duration::from_secs_f64(n as f64 / limit as f64);
        *next = Some(end);
        end.saturating_duration_since(now)
    };
    tokio::time::sleep(wait).await;
}

// ---------- retries ----------

/// Errors worth another try: network trouble and server hiccups, not "denied" or "not found".
pub fn transient(e: &str) -> bool {
    let l = e.to_lowercase();
    ![
        "cancelled",
        "denied",
        "forbidden",
        "not found",
        "no such",
        "already exists",
        "invalid",
        "unauthorized",
        "signature",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// Runs `f` up to 4 times when `enabled`, waiting 1 s, 3 s, 9 s between transient failures.
pub async fn retry<F, Fut>(enabled: bool, cancelled: impl Fn() -> bool, mut f: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let tries = if enabled { 4 } else { 1 };
    let mut wait = Duration::from_secs(1);
    for attempt in 1..=tries {
        match f().await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < tries && transient(&e) && !cancelled() => {
                tokio::time::sleep(wait).await;
                wait *= 3;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

// ---------- interrupted transfers ----------

/// A transfer as it was started, enough to start it again. Mirrors `PendingJob` in types.ts.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PendingJob {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub account: Option<String>,
    pub bucket: String,
    pub prefix: String,
    /// Uploads: local files and folders.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Downloads: remote items and the local folder.
    #[serde(default)]
    pub items: Vec<DlItem>,
    #[serde(default)]
    pub dest: Option<String>,
}

pub fn pending_add(p: PendingJob) {
    let _g = LOCK.lock().unwrap();
    let mut all: Vec<PendingJob> = load("pending.json");
    all.retain(|x| x.id != p.id);
    all.push(p);
    save("pending.json", &all);
}

#[cfg(test)]
pub fn pending_list() -> Vec<PendingJob> {
    load("pending.json")
}

pub fn pending_remove(id: &str) {
    let _g = LOCK.lock().unwrap();
    let mut all: Vec<PendingJob> = load("pending.json");
    let n = all.len();
    all.retain(|x| x.id != id);
    if all.len() != n {
        save("pending.json", &all);
    }
}

/// Transfers that didn't finish last time (the UI offers Resume / Dismiss).
#[tauri::command]
pub fn interrupted_jobs(state: State<'_, AppState>) -> Vec<PendingJob> {
    let _g = LOCK.lock().unwrap();
    let running = state.cancels.lock().unwrap();
    load::<Vec<PendingJob>>("pending.json").into_iter().filter(|p| !running.contains_key(&p.id)).collect()
}

#[tauri::command]
pub fn dismiss_interrupted(id: String) {
    pending_remove(&id);
}

/// Starts an interrupted transfer again (skipping what's already done). Returns the new job id.
#[tauri::command]
pub async fn resume_job(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<String, String> {
    let p = {
        let _g = LOCK.lock().unwrap();
        load::<Vec<PendingJob>>("pending.json").into_iter().find(|x| x.id == id).ok_or("Already resumed")?
    };
    pending_remove(&id);
    if p.kind == "up" {
        crate::transfer::upload(app, state, p.account, p.bucket, p.prefix, p.paths, Some(true)).await
    } else {
        crate::transfer::download(app, state, p.account, p.bucket, p.prefix, p.items, p.dest, Some(true)).await
    }
}

#[tauri::command]
pub fn pause(state: State<'_, AppState>, id: String, on: bool) {
    if let Some(f) = state.pauses.lock().unwrap().get(&id) {
        f.store(on, Ordering::Relaxed);
    }
}

// ---------- S3 multipart uploads in progress ----------

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Multipart {
    account: String,
    bucket: String,
    key: String,
    size: u64,
    /// Seconds since the epoch: a changed file must not continue an old upload.
    mtime: u64,
    upload_id: String,
}

fn mtime(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The unfinished multipart upload of this exact file, if one was journaled.
pub fn multipart_find(account: &str, bucket: &str, key: &str, size: u64, path: &std::path::Path) -> Option<String> {
    let _g = LOCK.lock().unwrap();
    let m = mtime(path);
    load::<Vec<Multipart>>("multipart.json")
        .into_iter()
        .find(|x| x.account == account && x.bucket == bucket && x.key == key && x.size == size && x.mtime == m)
        .map(|x| x.upload_id)
}

pub fn multipart_put(account: &str, bucket: &str, key: &str, size: u64, path: &std::path::Path, upload_id: &str) {
    let _g = LOCK.lock().unwrap();
    let mut all: Vec<Multipart> = load("multipart.json");
    all.retain(|x| !(x.account == account && x.bucket == bucket && x.key == key));
    all.push(Multipart {
        account: account.into(),
        bucket: bucket.into(),
        key: key.into(),
        size,
        mtime: mtime(path),
        upload_id: upload_id.into(),
    });
    save("multipart.json", &all);
}

pub fn multipart_remove(upload_id: &str) {
    let _g = LOCK.lock().unwrap();
    let mut all: Vec<Multipart> = load("multipart.json");
    let n = all.len();
    all.retain(|x| x.upload_id != upload_id);
    if all.len() != n {
        save("multipart.json", &all);
    }
}

/// Tests that change the global speed limit take turns.
#[cfg(test)]
pub(crate) static LIMIT_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors() {
        assert!(transient("Can't reach the server (check the address and your connection)"));
        assert!(transient("dispatch failure: timeout"));
        assert!(!transient("Access denied"));
        assert!(!transient("cancelled"));
        assert!(!transient("“a.txt” already exists there"));
    }

    #[tokio::test]
    async fn retry_stops_on_permanent_errors() {
        let mut n = 0;
        let r = retry(
            true,
            || false,
            || {
                n += 1;
                async { Err::<(), _>("Access denied".to_string()) }
            },
        )
        .await;
        assert!(r.is_err());
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn throttle_paces_bytes() {
        let _turn = LIMIT_TEST.lock().await;
        set_speed_limit(100); // 100 KB/s
        let t = Instant::now();
        for _ in 0..3 {
            throttle(10 * 1024).await; // 3 × 10 KB = 0.3 s
        }
        set_speed_limit(0);
        let el = t.elapsed().as_secs_f64();
        assert!((0.25..0.6).contains(&el), "{el}");
    }
}
