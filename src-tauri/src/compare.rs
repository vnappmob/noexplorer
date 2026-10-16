//! Compare a local folder with a remote one: what's missing on each side, what differs, and —
//! with `verify` — whether same-size files really have the same content (ADR-028).
//!
//! - Files are paired by their path relative to the two roots.
//! - Junk the uploader skips on purpose (`.DS_Store`, `Thumbs.db`, `desktop.ini`) and symlinks
//!   (never followed) are reported separately, so they don't look like lost files.
//! - Content checks never download anything: the local file is hashed and compared with what the
//!   server already reports — S3 ETags (plain MD5, or multipart MD5-of-MD5s: we try the part size
//!   our uploader used, then common ones from other tools), Box SHA-1, Nextcloud checksums.
//!   Files whose server hash can't be reproduced (e.g. SSE-KMS ETags) count as "same size".
//! - Progress goes out as `"compare-progress"` events; `cancel(id)` stops it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use futures::StreamExt;
use md5::{Digest as _, Md5};
use serde::Serialize;
use sha1::Sha1;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;
use crate::transfer::SKIP_NAMES;

/// Part sizes other tools use for multipart uploads (MiB): aws-cli/boto3 8, rclone 5, s3cmd 15…
const COMMON_PART_MIB: [u64; 10] = [5, 8, 10, 15, 16, 32, 50, 64, 100, 128];

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DiffFile {
    pub rel: String,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Changed {
    pub rel: String,
    pub local_size: u64,
    pub remote_size: u64,
    /// `size` or `content`.
    pub reason: &'static str,
}

/// Mirrors `Diff` in `src/types.ts`.
#[derive(Serialize, Default, Debug)]
pub struct Diff {
    pub local_files: u64,
    pub remote_files: u64,
    /// On this computer, not on the remote (would be uploaded).
    pub only_local: Vec<DiffFile>,
    /// On the remote, not here.
    pub only_remote: Vec<DiffFile>,
    pub changed: Vec<Changed>,
    /// Local junk the uploader skips on purpose.
    pub junk: Vec<DiffFile>,
    /// Local symlinks (not followed, not uploaded).
    pub symlinks: Vec<DiffFile>,
    /// Same size and the content hash matches.
    pub same_verified: u64,
    /// Same size, content not checked (not asked, or the server has no usable hash).
    pub same_size: u64,
    /// Local files that couldn't be read.
    pub errors: Vec<String>,
}

#[derive(Serialize, Clone)]
struct Progress {
    id: String,
    phase: &'static str,
    done: u64,
    total: u64,
}

/// Compares local folder `local` with remote folder `prefix` (in `bucket` of `account`).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn compare(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    account: Option<String>,
    bucket: String,
    prefix: String,
    local: String,
    verify: bool,
) -> Result<Diff, String> {
    let remote = state.remote(account.as_deref())?;
    let cancel = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(id.clone(), cancel.clone());
    let emit = |phase: &'static str, done: u64, total: u64| {
        let _ = app.emit("compare-progress", Progress { id: id.clone(), phase, done, total });
    };
    let result = diff(&remote, &bucket, &prefix, &local, verify, &cancel, &emit).await;
    state.cancels.lock().unwrap().remove(&id);
    emit("done", 0, 0);
    result
}

/// The comparison itself (see the module docs). `emit(phase, done, total)` reports progress.
pub async fn diff(
    remote: &crate::provider::Remote,
    bucket: &str,
    prefix: &str,
    local: &str,
    verify: bool,
    cancel: &Arc<AtomicBool>,
    emit: &(dyn Fn(&'static str, u64, u64) + Sync),
) -> Result<Diff, String> {
    {
        emit("listing", 0, 0);
        let root = PathBuf::from(local);
        let walk = tokio::task::spawn_blocking(move || walk_local(&root));
        let remote_files = remote.list_files(bucket, prefix).await?;
        let (files, junk, symlinks) = walk.await.map_err(|e| e.to_string())??;
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }
        let remote_map: HashMap<String, (u64, Option<String>)> = remote_files
            .into_iter()
            .filter_map(|e| e.key.strip_prefix(prefix).map(|r| (r.to_string(), (e.size, e.hash))))
            .collect();
        let mut diff = pair(&files, &remote_map, junk, symlinks);

        // Same size on both sides: check content where the server gives us something to check against.
        let todo: Vec<(String, u64, String)> = if verify {
            files
                .iter()
                .filter_map(|(rel, size)| {
                    let (rsize, hash) = remote_map.get(rel)?;
                    (rsize == size).then(|| hash.clone().map(|h| (rel.clone(), *size, h))).flatten()
                })
                .collect()
        } else {
            vec![]
        };
        let total: u64 = todo.iter().map(|t| t.1).sum();
        let done = Arc::new(AtomicU64::new(0));
        let ticker = {
            let (done, cancel) = (done.clone(), cancel.clone());
            async move {
                loop {
                    emit("hashing", done.load(Ordering::Relaxed), total);
                    if cancel.load(Ordering::Relaxed) || done.load(Ordering::Relaxed) >= total {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        };
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 6);
        let checks = futures::stream::iter(todo)
            .map(|(rel, size, hash)| {
                let (path, done, cancel) = (Path::new(local).join(&rel), done.clone(), cancel.clone());
                async move {
                    let v = tokio::task::spawn_blocking(move || matches_hash(&path, size, &hash, &done, &cancel))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    (rel, size, v)
                }
            })
            .buffer_unordered(threads)
            .collect::<Vec<_>>();
        let (results, _) = futures::join!(checks, ticker);
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }
        for (rel, size, v) in results {
            match v {
                Ok(Some(true)) => {
                    diff.same_verified += 1;
                    diff.same_size -= 1;
                }
                Ok(Some(false)) => {
                    diff.same_size -= 1;
                    diff.changed.push(Changed { rel, local_size: size, remote_size: size, reason: "content" });
                }
                Ok(None) => {} // nothing to compare against: stays "same size"
                Err(e) => diff.errors.push(format!("{rel}: {e}")),
            }
        }
        diff.changed.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(diff)
    }
}

type Walk = (Vec<(String, u64)>, Vec<DiffFile>, Vec<DiffFile>);

/// Files under `root` as (`a/b.txt`, size), plus junk and symlinks.
fn walk_local(root: &Path) -> Result<Walk, String> {
    if !root.is_dir() {
        return Err(format!("{}: not a folder", root.display()));
    }
    let (mut files, mut junk, mut links) = (vec![], vec![], vec![]);
    for e in walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_map(Result::ok) {
        let rel: Vec<String> = match e.path().strip_prefix(root) {
            Ok(r) if !r.as_os_str().is_empty() => {
                r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()
            }
            _ => continue,
        };
        let rel = rel.join("/");
        let ft = e.file_type();
        if ft.is_symlink() {
            links.push(DiffFile { rel, size: 0 });
        } else if ft.is_file() {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            if SKIP_NAMES.contains(&e.file_name().to_string_lossy().as_ref()) {
                junk.push(DiffFile { rel, size });
            } else {
                files.push((rel, size));
            }
        }
    }
    Ok((files, junk, links))
}

/// Pairs local files with remote ones by relative path (no content checks yet).
fn pair(
    files: &[(String, u64)],
    remote: &HashMap<String, (u64, Option<String>)>,
    junk: Vec<DiffFile>,
    symlinks: Vec<DiffFile>,
) -> Diff {
    let mut d = Diff {
        local_files: files.len() as u64,
        remote_files: remote.len() as u64,
        junk,
        symlinks,
        ..Default::default()
    };
    let local: HashMap<&str, u64> = files.iter().map(|(r, s)| (r.as_str(), *s)).collect();
    for (rel, size) in files {
        match remote.get(rel) {
            None => d.only_local.push(DiffFile { rel: rel.clone(), size: *size }),
            Some((rs, _)) if rs != size => {
                d.changed.push(Changed { rel: rel.clone(), local_size: *size, remote_size: *rs, reason: "size" })
            }
            Some(_) => d.same_size += 1,
        }
    }
    for (rel, (size, _)) in remote {
        if !local.contains_key(rel.as_str()) {
            d.only_remote.push(DiffFile { rel: rel.clone(), size: *size });
        }
    }
    d.only_local.sort_by(|a, b| a.rel.cmp(&b.rel));
    d.only_remote.sort_by(|a, b| a.rel.cmp(&b.rel));
    d.junk.sort_by(|a, b| a.rel.cmp(&b.rel));
    d
}

/// Does the local file match the server's `algo:value` hash? None = can't tell.
fn matches_hash(
    path: &Path,
    size: u64,
    hash: &str,
    done: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<Option<bool>, String> {
    let (algo, value) = hash.split_once(':').ok_or("bad hash")?;
    let value = value.to_ascii_lowercase();
    match algo {
        "sha1" => Ok(Some(hex(&digest_file::<Sha1>(path, done, cancel)?) == value)),
        "md5" => Ok(Some(hex(&digest_file::<Md5>(path, done, cancel)?) == value)),
        "etag" => match value.split_once('-') {
            // Plain PUT: the ETag is the MD5 (unless the server encrypts with KMS — not hex-32 then).
            None if value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Ok(Some(hex(&digest_file::<Md5>(path, done, cancel)?) == value))
            }
            None => Ok(None),
            Some((_, parts)) => {
                let parts: u64 = parts.parse().map_err(|_| "bad ETag")?;
                let candidates = part_size_candidates(size, parts);
                for ps in &candidates {
                    if multipart_etag(path, *ps, done, cancel)? == value {
                        return Ok(Some(true));
                    }
                }
                // Our own part size reproduces any upload NoExplorer made: a mismatch means the content
                // differs. Without it, another tool used a part size we can't guess: can't tell.
                Ok(candidates.contains(&our_part_size(size)).then_some(false))
            }
        },
        _ => Ok(None),
    }
}

/// Part sizes that split `size` into exactly `parts`: ours first, then common ones.
fn part_size_candidates(size: u64, parts: u64) -> Vec<u64> {
    let mut out: Vec<u64> = std::iter::once(our_part_size(size))
        .chain(COMMON_PART_MIB.iter().map(|m| m * 1024 * 1024))
        .filter(|ps| size.div_ceil(*ps) == parts)
        .collect();
    out.dedup();
    out
}

/// The part size `s3.rs` uses for a file of `size` bytes.
fn our_part_size(size: u64) -> u64 {
    crate::s3::MIN_PART_SIZE.max(size.div_ceil(crate::s3::MAX_PARTS))
}

/// S3 multipart ETag: MD5 of the concatenated part MD5s, then `-<parts>`.
fn multipart_etag(path: &Path, part: u64, done: &AtomicU64, cancel: &AtomicBool) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut all = Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let mut h = Md5::new();
        let mut left = part;
        let mut read_any = false;
        while left > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            let want = left.min(buf.len() as u64) as usize;
            let k = f.read(&mut buf[..want]).map_err(|e| e.to_string())?;
            if k == 0 {
                break;
            }
            read_any = true;
            h.update(&buf[..k]);
            left -= k as u64;
            done.fetch_add(k as u64, Ordering::Relaxed);
        }
        if !read_any {
            break;
        }
        all.update(h.finalize());
        n += 1;
    }
    Ok(format!("{}-{n}", hex(&all.finalize())))
}

fn digest_file<D: sha1::Digest>(path: &Path, done: &AtomicU64, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let k = f.read(&mut buf).map_err(|e| e.to_string())?;
        if k == 0 {
            return Ok(h.finalize().to_vec());
        }
        h.update(&buf[..k]);
        done.fetch_add(k as u64, Ordering::Relaxed);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Profile;
    use crate::jobs::Job;
    use crate::provider::Remote;

    #[test]
    fn pairs_and_separates_junk() {
        let remote = HashMap::from([
            ("a.txt".to_string(), (3, None)),
            ("b.txt".to_string(), (9, None)),
            ("gone-here.txt".to_string(), (1, None)),
        ]);
        let files = vec![("a.txt".to_string(), 3), ("b.txt".to_string(), 4), ("new.txt".to_string(), 2)];
        let d = pair(&files, &remote, vec![DiffFile { rel: ".DS_Store".into(), size: 6 }], vec![]);
        assert_eq!((d.local_files, d.remote_files, d.same_size), (3, 3, 1));
        assert_eq!(d.only_local, vec![DiffFile { rel: "new.txt".into(), size: 2 }]);
        assert_eq!(d.only_remote, vec![DiffFile { rel: "gone-here.txt".into(), size: 1 }]);
        assert_eq!(d.changed[0].reason, "size");
        assert_eq!(d.junk.len(), 1);
        let mib = 1024 * 1024;
        assert_eq!(part_size_candidates(40 * mib, 5), vec![8 * mib], "ours (8 MB) splits 40 MB into 5");
        assert_eq!(part_size_candidates(40 * mib, 4), vec![10 * mib]);
        assert!(part_size_candidates(40 * mib, 7).is_empty());
    }

    /// Against the dev S3 server (see `transfer::tests::s3_roundtrip` for the variables).
    #[tokio::test]
    async fn compare_s3_roundtrip() {
        let Ok(endpoint) = std::env::var("NOEXPLORER_TEST_ENDPOINT") else {
            eprintln!("skipping compare_s3_roundtrip: NOEXPLORER_TEST_ENDPOINT not set");
            return;
        };
        let cfg = Profile {
            endpoint,
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            ..Default::default()
        };
        let bucket = "noexplorer-test";
        let _ = crate::s3::make_client(&cfg).create_bucket().bucket(bucket).send().await;
        let r = Remote::new(&cfg);
        let dir = std::env::temp_dir().join(format!("nx-cmp-{}", uuid::Uuid::new_v4()));
        let root = dir.join("proj");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let big: Vec<u8> = (0..(20 * 1024 * 1024 + 5)).map(|i| (i % 253) as u8).collect();
        std::fs::write(root.join("big.bin"), &big).unwrap(); // multipart: 3 parts of 8 MB
        for (p, c) in [
            ("same.txt", "same"),
            ("edit.txt", "aaaa"),
            ("grow.txt", "x"),
            ("sub/gone.txt", "bye"),
            ("sub/.DS_Store", "junk"),
        ] {
            std::fs::write(root.join(p), c).unwrap();
        }
        let job = Job::untracked("up", "t".into(), "", bucket, "");
        let prefix = format!("cmp-{}/proj/", uuid::Uuid::new_v4().simple());
        for (path, key, size) in
            crate::transfer::collect_uploads(&[root.to_string_lossy().into()], &prefix[..prefix.len() - 5])
        {
            r.upload_file(bucket, &key, &path, size, &job).await.unwrap();
        }
        // Drift: same-size edit, size change, a file only remote, a file deleted remotely.
        std::fs::write(root.join("edit.txt"), "bbbb").unwrap();
        std::fs::write(root.join("grow.txt"), "xyz").unwrap();
        r.delete(bucket, vec![format!("{prefix}sub/gone.txt")]).await.unwrap();
        r.upload_file(bucket, &format!("{prefix}remote-only.txt"), &root.join("same.txt"), 4, &job).await.unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let quiet = |_: &'static str, _: u64, _: u64| {};
        let d = diff(&r, bucket, &prefix, &root.to_string_lossy(), true, &cancel, &quiet).await.unwrap();
        assert_eq!(d.only_local.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), ["sub/gone.txt"]);
        assert_eq!(d.only_remote.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), ["remote-only.txt"]);
        let changed: Vec<(&str, &str)> = d.changed.iter().map(|c| (c.rel.as_str(), c.reason)).collect();
        assert_eq!(changed, [("edit.txt", "content"), ("grow.txt", "size")]);
        assert_eq!(d.junk.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), ["sub/.DS_Store"]);
        assert_eq!((d.same_verified, d.same_size), (2, 0), "big.bin (multipart ETag) and same.txt verified");

        r.delete(bucket, vec![prefix.split('/').next().unwrap().to_string() + "/"]).await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
