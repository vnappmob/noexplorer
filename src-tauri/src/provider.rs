//! Remote storage behind one interface (ADR-025). Commands in `remote.rs` / `transfer.rs` talk to
//! a [`Remote`], never to a provider's SDK directly.
//!
//! - One `Remote` per account, built from its `Profile` and cached in `AppState` (no single
//!   "active client": every command names its account, so several accounts can be open at once).
//! - `Remote` is an enum with static dispatch (no `dyn`, no async-trait): adding a provider means
//!   adding a variant and filling in each `match`.
//! - Locations are `(container, key)`. S3: container = bucket. Folder keys end with `/`, and keys
//!   are `/`-separated paths relative to the container.

use std::path::Path;

use serde::Serialize;

use crate::config::{Profile, ProviderKind};
use crate::jobs::Job;
use crate::s3::S3;

/// One row in a pane. Mirrors `Item` in `src/types.ts` (local entries use the same shape).
#[derive(Serialize)]
pub struct Entry {
    /// Full key; folders end with `/`.
    pub key: String,
    pub name: String,
    pub size: u64,
    /// Unix millis.
    pub modified: Option<i64>,
    pub dir: bool,
}

/// A top-level container (an S3 bucket). Mirrors `BucketInfo` in `src/types.ts`.
#[derive(Serialize)]
pub struct BucketInfo {
    pub name: String,
    /// Known on AWS (from ListBuckets); None for most self-hosted servers.
    pub region: Option<String>,
}

pub enum Remote {
    S3(S3),
}

impl Remote {
    pub fn new(p: &Profile) -> Remote {
        match p.kind {
            ProviderKind::S3 => Remote::S3(S3::new(p)),
        }
    }

    /// The profile id of this account.
    pub fn account(&self) -> &str {
        match self {
            Remote::S3(s) => s.account(),
        }
    }

    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        match self {
            Remote::S3(s) => s.containers().await,
        }
    }

    /// One folder level.
    pub async fn list(&self, container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        match self {
            Remote::S3(s) => s.list(container, prefix).await,
        }
    }

    /// Every file under `prefix` (recursive) as `(key, size)`; may include folder markers (keys
    /// ending in `/`).
    pub async fn list_all(&self, container: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        match self {
            Remote::S3(s) => s.list_all(container, prefix).await,
        }
    }

    /// A time-limited URL for `key` (previews with `inline`, "Copy link").
    pub async fn link(&self, container: &str, key: &str, secs: u64, inline: bool) -> Result<String, String> {
        match self {
            Remote::S3(s) => s.link(container, key, secs, inline).await,
        }
    }

    /// The first `max` bytes of a file.
    pub async fn read_head(&self, container: &str, key: &str, max: u64) -> Result<Vec<u8>, String> {
        match self {
            Remote::S3(s) => s.read_head(container, key, max).await,
        }
    }

    pub async fn mkdir(&self, container: &str, prefix: &str, name: &str) -> Result<(), String> {
        match self {
            Remote::S3(s) => s.mkdir(container, prefix, name).await,
        }
    }

    /// Deletes files and folders (keys ending in `/`, recursively). Returns how many objects went.
    pub async fn delete(&self, container: &str, keys: Vec<String>) -> Result<usize, String> {
        match self {
            Remote::S3(s) => s.delete(container, keys).await,
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
        }
    }

    /// Streams a file to `path` (through `path.part`, renamed on success).
    pub async fn download_file(&self, container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        match self {
            Remote::S3(s) => s.download_file(container, key, path, job).await,
        }
    }
}
