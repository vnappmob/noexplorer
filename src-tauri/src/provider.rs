//! Remote storage behind one interface (ADR-025). Commands in `remote.rs` / `transfer.rs` talk to
//! a [`Remote`], never to a provider's SDK directly.
//!
//! - One `Remote` per account, built from its `Profile` and cached in `AppState` (no single
//!   "active client": every command names its account, so several accounts can be open at once).
//! - `Remote` is an enum with static dispatch (no `dyn`, no async-trait): adding a provider means
//!   adding a variant and filling in each `match`.
//! - Locations are `(container, key)`. S3: container = bucket; WebDAV: one container ("Files").
//!   Folder keys end with `/`, and keys are `/`-separated paths relative to the container.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::azure::Azure;
use crate::box_api::BoxCom;
use crate::config::{Profile, ProviderKind};
use crate::jobs::Job;
use crate::onedrive::OneDrive;
use crate::s3::S3;
use crate::sftp::Sftp;
use crate::webdav::Dav;

/// One row in a pane. Mirrors `Item` in `src/types.ts` (local entries use the same shape).
#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    /// Full key; folders end with `/`.
    pub key: String,
    pub name: String,
    pub size: u64,
    /// Unix millis.
    pub modified: Option<i64>,
    pub dir: bool,
    /// Content hash the server reports, as `algo:value` (`etag:…` S3, `sha1:…` Box/Nextcloud,
    /// `md5:…`). Backend-only: used by Compare (`compare.rs`).
    #[serde(skip)]
    pub hash: Option<String>,
}

/// A top-level container (an S3 bucket). Mirrors `BucketInfo` in `src/types.ts`.
#[derive(Serialize)]
pub struct BucketInfo {
    pub name: String,
    /// Known on AWS (from ListBuckets); None for most self-hosted servers.
    pub region: Option<String>,
}

const NO_POLICIES: &str = "Only S3 buckets have access policies";

pub enum Remote {
    S3(S3),
    /// Nextcloud and plain WebDAV (`webdav.rs`).
    Dav(Dav),
    /// Box (`box_api.rs`).
    Box(BoxCom),
    /// Azure Blob Storage (`azure.rs`).
    Azure(Azure),
    /// SFTP through the system `ssh` (`sftp.rs`).
    Sftp(Sftp),
    /// OneDrive through Microsoft Graph (`onedrive.rs`).
    OneDrive(OneDrive),
}

impl Remote {
    pub fn new(p: &Profile) -> Remote {
        match p.kind {
            ProviderKind::S3 => Remote::S3(S3::new(p)),
            ProviderKind::Nextcloud | ProviderKind::Webdav => Remote::Dav(Dav::new(p)),
            ProviderKind::Box => Remote::Box(BoxCom::new(p)),
            ProviderKind::Azure => Remote::Azure(Azure::new(p)),
            ProviderKind::Sftp => Remote::Sftp(Sftp::new(p)),
            ProviderKind::Onedrive => Remote::OneDrive(OneDrive::new(p)),
        }
    }

    /// How many files a transfer moves at once. OneDrive throttles a user's parallel requests
    /// (429 + Retry-After) well before the line is full, so a few files at a time finish sooner.
    pub fn file_concurrency(&self) -> usize {
        match self {
            Remote::OneDrive(_) => 3,
            _ => crate::transfer::FILE_CONCURRENCY,
        }
    }

    /// The profile id of this account.
    pub fn account(&self) -> &str {
        match self {
            Remote::S3(s) => s.account(),
            Remote::Dav(d) => d.account(),
            Remote::Box(b) => b.account(),
            Remote::OneDrive(o) => o.account(),
            Remote::Azure(a) => a.account(),
            Remote::Sftp(f) => f.account(),
        }
    }

    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        match self {
            Remote::S3(s) => s.containers().await,
            Remote::Dav(d) => d.containers().await,
            Remote::Box(b) => b.containers().await,
            Remote::OneDrive(o) => o.containers().await,
            Remote::Azure(a) => a.containers().await,
            Remote::Sftp(f) => f.containers().await,
        }
    }

    /// Creates a bucket (S3) or container (Azure). The other providers have one fixed container.
    pub async fn create_container(&self, name: &str) -> Result<(), String> {
        match self {
            Remote::S3(s) => {
                crate::util::check_bucket_name(name, false)?;
                s.create_bucket(name).await
            }
            Remote::Azure(a) => {
                crate::util::check_bucket_name(name, true)?;
                a.create_container(name).await
            }
            _ => Err("This storage has no buckets to create".into()),
        }
    }

    /// The bucket's access policy (S3 only; None = no policy).
    pub async fn policy(&self, container: &str) -> Result<Option<String>, String> {
        match self {
            Remote::S3(s) => s.policy(container).await,
            _ => Err(NO_POLICIES.into()),
        }
    }

    pub async fn set_policy(&self, container: &str, policy: Option<&str>) -> Result<(), String> {
        match self {
            Remote::S3(s) => s.set_policy(container, policy).await,
            _ => Err(NO_POLICIES.into()),
        }
    }

    /// A file's public (unsigned) address. S3 only.
    pub async fn public_url(&self, container: &str, key: &str) -> Result<String, String> {
        match self {
            Remote::S3(s) => Ok(s.public_url(container, key).await),
            _ => Err(NO_POLICIES.into()),
        }
    }

    /// Every file below `prefix` (no folders or folder markers), with server hashes where the
    /// provider has them (for Compare).
    pub async fn list_files(&self, container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        match self {
            Remote::S3(s) => s.list_files(container, prefix).await,
            Remote::Dav(d) => walk_files(prefix, |dir| async move { d.list(container, &dir).await }).await,
            Remote::Box(b) => walk_files(prefix, |dir| async move { b.list(container, &dir).await }).await,
            Remote::OneDrive(o) => walk_files(prefix, |dir| async move { o.list(container, &dir).await }).await,
            Remote::Azure(a) => a.list_files(container, prefix).await,
            Remote::Sftp(f) => walk_files(prefix, |dir| async move { f.list(container, &dir).await }).await,
        }
    }

    /// One folder level.
    pub async fn list(&self, container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        match self {
            Remote::S3(s) => s.list(container, prefix).await,
            Remote::Dav(d) => d.list(container, prefix).await,
            Remote::Box(b) => b.list(container, prefix).await,
            Remote::OneDrive(o) => o.list(container, prefix).await,
            Remote::Azure(a) => a.list(container, prefix).await,
            Remote::Sftp(f) => f.list(container, prefix).await,
        }
    }

    /// Every file under `prefix` (recursive) as `(key, size)`; may include folder markers (keys
    /// ending in `/`).
    pub async fn list_all(&self, container: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        match self {
            Remote::S3(s) => s.list_all(container, prefix).await,
            Remote::Dav(d) => d.list_all(container, prefix).await,
            Remote::Box(b) => b.list_all(container, prefix).await,
            Remote::OneDrive(o) => o.list_all(container, prefix).await,
            Remote::Azure(a) => a.list_all(container, prefix).await,
            Remote::Sftp(f) => Ok(walk_files(prefix, |dir| async move { f.list(container, &dir).await })
                .await?
                .into_iter()
                .map(|e| (e.key, e.size))
                .collect()),
        }
    }

    /// A time-limited URL for `key` (previews with `inline`, "Copy link").
    pub async fn link(&self, container: &str, key: &str, secs: u64, inline: bool) -> Result<String, String> {
        match self {
            Remote::S3(s) => s.link(container, key, secs, inline).await,
            Remote::Dav(d) => d.link(container, key, secs, inline).await,
            Remote::Box(b) => b.link(container, key, secs, inline).await,
            Remote::OneDrive(o) => o.link(container, key, secs, inline).await,
            Remote::Azure(a) => a.link(container, key, secs, inline).await,
            // Previews stream through the app (proxy.rs); SFTP has no share links.
            Remote::Sftp(f) if inline => Ok(crate::proxy::url(f.account(), container, key)),
            Remote::Sftp(_) => Err("SFTP can't make share links".into()),
        }
    }

    /// The first `max` bytes of a file.
    pub async fn read_head(&self, container: &str, key: &str, max: u64) -> Result<Vec<u8>, String> {
        match self {
            Remote::S3(s) => s.read_head(container, key, max).await,
            Remote::Dav(d) => d.read_head(container, key, max).await,
            Remote::Box(b) => b.read_head(container, key, max).await,
            Remote::OneDrive(o) => o.read_head(container, key, max).await,
            Remote::Azure(a) => a.read_head(container, key, max).await,
            Remote::Sftp(f) => f.read_head(key, max).await,
        }
    }

    /// Bytes `start..=end` (open-ended: at most `cap` bytes) and the file's total size, for the
    /// preview proxy (`proxy.rs`).
    pub async fn read_range(
        &self,
        container: &str,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        match self {
            Remote::S3(s) => s.read_range(container, key, start, end, cap).await,
            Remote::Dav(d) => d.read_range(key, start, end, cap).await,
            Remote::Box(b) => b.read_range(key, start, end, cap).await,
            Remote::OneDrive(o) => o.read_range(key, start, end, cap).await,
            Remote::Azure(a) => a.read_range(container, key, start, end, cap).await,
            Remote::Sftp(f) => f.read_range(key, start, end, cap).await,
        }
    }

    /// Copies a file inside this account without downloading it, when the provider can.
    /// `Ok(false)` = not supported here (the caller relays the bytes instead).
    pub async fn copy_within(&self, src: (&str, &str), dst: (&str, &str), size: u64) -> Result<bool, String> {
        match self {
            Remote::S3(s) if size <= 5 * 1024 * 1024 * 1024 => {
                s.copy_within(src.0, src.1, dst.0, dst.1).await.map(|_| true)
            }
            _ => Ok(false),
        }
    }

    pub async fn mkdir(&self, container: &str, prefix: &str, name: &str) -> Result<(), String> {
        match self {
            Remote::S3(s) => s.mkdir(container, prefix, name).await,
            Remote::Dav(d) => d.mkdir(container, prefix, name).await,
            Remote::Box(b) => b.mkdir(container, prefix, name).await,
            Remote::OneDrive(o) => o.mkdir(container, prefix, name).await,
            Remote::Azure(a) => a.mkdir(container, prefix, name).await,
            Remote::Sftp(f) => f.mkdir(container, prefix, name).await,
        }
    }

    /// Deletes files and folders (keys ending in `/`, recursively). Returns how many objects went.
    /// Copies or moves (`keep` = copy) a file or folder to `to` within one container, on the
    /// server. Never overwrites: an existing target is an error.
    pub async fn transfer_within(
        &self,
        container: &str,
        from: &str,
        to: &str,
        keep: bool,
        job: Option<&Job>,
    ) -> Result<(), String> {
        if from.ends_with('/') && to.starts_with(from) {
            return Err("A folder can't go inside itself".into());
        }
        match self {
            Remote::S3(s) => s.transfer_within(container, from, to, keep, job).await,
            Remote::Dav(d) => d.transfer_within(container, from, to, keep, job).await,
            Remote::Box(b) => b.transfer_within(container, from, to, keep, job).await,
            Remote::OneDrive(o) => o.transfer_within(container, from, to, keep, job).await,
            Remote::Azure(a) => a.transfer_within(container, from, to, keep, job).await,
            Remote::Sftp(f) => f.transfer_within(container, from, to, keep, job).await,
        }
    }

    /// Deletes files and folders (with everything inside). `job` gets the progress (files =
    /// objects or items) and can cancel between steps.
    pub async fn delete(&self, container: &str, keys: Vec<String>, job: Option<&Job>) -> Result<usize, String> {
        match self {
            Remote::S3(s) => s.delete(container, keys, job).await,
            Remote::Dav(d) => d.delete(container, keys, job).await,
            Remote::Box(b) => b.delete(container, keys, job).await,
            Remote::OneDrive(o) => o.delete(container, keys, job).await,
            Remote::Azure(a) => a.delete(container, keys, job).await,
            Remote::Sftp(f) => f.delete(container, keys, job).await,
        }
    }

    pub async fn upload_file(
        &self,
        container: &str,
        key: &str,
        path: &Path,
        size: u64,
        job: &Job,
    ) -> Result<(), String> {
        match self {
            Remote::S3(s) => s.upload_file(container, key, path, size, job).await,
            Remote::Dav(d) => d.upload_file(container, key, path, size, job).await,
            Remote::Box(b) => b.upload_file(container, key, path, size, job).await,
            Remote::OneDrive(o) => o.upload_file(container, key, path, size, job).await,
            Remote::Azure(a) => a.upload_file(container, key, path, size, job).await,
            Remote::Sftp(f) => f.upload_file(container, key, path, size, job).await,
        }
    }

    /// Streams a file to `path` (through `path.part`, renamed on success).
    pub async fn download_file(&self, container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        match self {
            Remote::S3(s) => s.download_file(container, key, path, job).await,
            Remote::Dav(d) => d.download_file(container, key, path, job).await,
            Remote::Box(b) => b.download_file(container, key, path, job).await,
            Remote::OneDrive(o) => o.download_file(container, key, path, job).await,
            Remote::Azure(a) => a.download_file(container, key, path, job).await,
            Remote::Sftp(f) => f.download_file(container, key, path, job).await,
        }
    }
}

/// Walks folders level by level (a few at a time) and returns the files.
async fn walk_files<F, Fut>(prefix: &str, list: F) -> Result<Vec<Entry>, String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<Entry>, String>>,
{
    use futures::StreamExt;
    let mut out = vec![];
    let mut level = vec![prefix.to_string()];
    while !level.is_empty() {
        let lists: Vec<Result<Vec<Entry>, String>> =
            futures::stream::iter(level.drain(..).map(&list)).buffer_unordered(4).collect().await;
        for l in lists {
            for e in l? {
                if e.dir {
                    level.push(e.key);
                } else {
                    out.push(e);
                }
            }
        }
    }
    Ok(out)
}

// ---------- `.part` files for downloads (shared by every provider) ----------

/// Where a download of `path` is written until it's complete (then renamed, ADR-006).
pub(crate) fn part_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    PathBuf::from(tmp)
}

/// Bytes already in the `.part` to continue from: only for resumable jobs.
pub(crate) async fn resume_offset(tmp: &Path, job: &Job) -> u64 {
    if !job.resume {
        return 0;
    }
    tokio::fs::metadata(tmp).await.map(|m| m.len()).unwrap_or(0)
}

/// Opens the `.part` for writing: appended to when the server honored the range (`partial`),
/// otherwise from scratch. Bytes kept from before count as progress.
pub(crate) async fn open_part(tmp: &Path, partial: bool, offset: u64, job: &Job) -> Result<tokio::fs::File, String> {
    if partial {
        job.bytes_done.fetch_add(offset, std::sync::atomic::Ordering::Relaxed);
        tokio::fs::OpenOptions::new().append(true).open(tmp).await.map_err(|e| e.to_string())
    } else {
        tokio::fs::File::create(tmp).await.map_err(|e| e.to_string())
    }
}

/// After a failed download: keep the `.part` when it can be resumed (network trouble in a resumable
/// job), otherwise remove it.
pub(crate) async fn drop_part(tmp: &Path, job: &Job, err: &str) {
    if !(job.resume && !job.cancelled() && crate::resume::transient(err)) {
        let _ = tokio::fs::remove_file(tmp).await;
    }
}
