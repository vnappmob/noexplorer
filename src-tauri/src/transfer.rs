//! Upload / download / open jobs, provider-agnostic (ADR-025): each job holds its account's
//! `Remote` and moves up to [`FILE_CONCURRENCY`] files at once (ADR-005). How a single file moves
//! (multipart, chunking, `.part` files) is the provider's business, see `s3.rs`.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use futures::{StreamExt, stream};
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::jobs::Job;
use crate::provider::Remote;
use crate::state::AppState;
use crate::util::{file_name, job_label, safe_rel, unique_path};

pub const FILE_CONCURRENCY: usize = 6;

/// Junk files never uploaded from folders.
pub const SKIP_NAMES: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// Temp folder for "Open with default app" downloads; wiped at startup.
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("noexplorer")
}

/// Expands dropped paths into `(local path, object key, size)`. Folders are walked
/// recursively and keep their structure under `prefix` (`prefix + folder/sub/file`).
pub fn collect_uploads(paths: &[String], prefix: &str) -> Vec<(PathBuf, String, u64)> {
    let skip = |n: &str| SKIP_NAMES.contains(&n);
    let mut out = vec![];
    for p in paths {
        let p = PathBuf::from(p);
        let Some(base) = p.file_name().map(|s| s.to_string_lossy().to_string()) else { continue };
        if p.is_dir() {
            for e in walkdir::WalkDir::new(&p).into_iter().filter_map(Result::ok) {
                if !e.file_type().is_file() || skip(&e.file_name().to_string_lossy()) {
                    continue;
                }
                let rel: Vec<String> = e
                    .path()
                    .strip_prefix(&p)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect();
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                out.push((e.path().to_path_buf(), format!("{prefix}{base}/{}", rel.join("/")), size));
            }
        } else if p.is_file() && !skip(&base) {
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((p, format!("{prefix}{base}"), size));
        }
    }
    out
}

/// Starts an upload job for local files/folders into `bucket/prefix`. Returns the job id.
#[tauri::command]
pub async fn upload(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    paths: Vec<String>,
    resume: Option<bool>,
) -> Result<String, String> {
    let c = state.remote(account.as_deref())?;
    let label = job_label(paths.iter().map(|p| file_name(&p.replace('\\', "/"))).collect());
    let job = Job::new(&app, "up", label.clone(), c.account(), &bucket, &prefix);
    if job.resume {
        crate::resume::pending_add(crate::resume::PendingJob {
            id: job.id.clone(),
            kind: "up".into(),
            label,
            account: account.clone(),
            bucket: bucket.clone(),
            prefix: prefix.clone(),
            paths: paths.clone(),
            items: vec![],
            dest: None,
        });
    }
    let resuming = resume.unwrap_or(false);
    *job.src_dir.lock().unwrap() =
        paths.first().and_then(|p| Path::new(p).parent()).map(|d| d.to_string_lossy().into_owned());
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        let roots: Vec<String> = paths
            .iter()
            .map(|p| {
                let base = file_name(&p.replace('\\', "/"));
                if Path::new(p).is_dir() { format!("{prefix}{base}/") } else { format!("{prefix}{base}") }
            })
            .collect();
        let mut files = tokio::task::spawn_blocking(move || collect_uploads(&paths, &prefix)).await.unwrap_or_default();
        j.files_total.store(files.len() as u64, Ordering::Relaxed);
        j.bytes_total.store(files.iter().map(|f| f.2).sum(), Ordering::Relaxed);
        if resuming {
            // Resuming an interrupted upload: what's already there with the same size is done.
            let mut have = HashMap::new();
            for r in &roots {
                for e in c.list_files(&bucket, r).await.unwrap_or_default() {
                    have.insert(e.key, e.size);
                }
            }
            files.retain(|(_, k, s)| {
                let done = have.get(k) == Some(s);
                if done {
                    j.files_done.fetch_add(1, Ordering::Relaxed);
                    j.bytes_done.fetch_add(*s, Ordering::Relaxed);
                }
                !done
            });
        }
        stream::iter(files)
            .for_each_concurrent(FILE_CONCURRENCY, |(path, key, size)| {
                let (c, j, bucket) = (c.clone(), j.clone(), bucket.clone());
                async move {
                    if j.cancelled() {
                        return;
                    }
                    let up = || c.upload_file(&bucket, &key, &path, size, &j);
                    match crate::resume::retry(j.resume, || j.cancelled(), up).await {
                        Ok(()) => {
                            j.files_done.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) if !j.cancelled() => j.fail(&file_name(&key), e),
                        Err(_) => {}
                    }
                }
            })
            .await;
    });
    Ok(id)
}

/// Uploads files given relative to a local `root` into `prefix` + the same relative path
/// (Compare → "Upload missing/changed"). Returns the job id.
#[tauri::command]
pub async fn upload_rel(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    root: String,
    rels: Vec<String>,
) -> Result<String, String> {
    let c = state.remote(account.as_deref())?;
    let label = job_label(rels.iter().map(|r| file_name(r)).collect());
    let job = Job::new(&app, "up", label, c.account(), &bucket, &prefix);
    *job.src_dir.lock().unwrap() = Some(root.clone());
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        let root = PathBuf::from(root);
        let files: Vec<(PathBuf, String, u64)> = rels
            .iter()
            .filter_map(|rel| {
                let path = root.join(safe_rel(rel));
                let size = std::fs::metadata(&path).ok().filter(|m| m.is_file())?.len();
                Some((path, format!("{prefix}{}", rel.trim_start_matches('/')), size))
            })
            .collect();
        j.files_total.store(files.len() as u64, Ordering::Relaxed);
        j.bytes_total.store(files.iter().map(|f| f.2).sum(), Ordering::Relaxed);
        stream::iter(files)
            .for_each_concurrent(FILE_CONCURRENCY, |(path, key, size)| {
                let (c, j, bucket) = (c.clone(), j.clone(), bucket.clone());
                async move {
                    if j.cancelled() {
                        return;
                    }
                    match c.upload_file(&bucket, &key, &path, size, &j).await {
                        Ok(()) => {
                            j.files_done.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) if !j.cancelled() => j.fail(&file_name(&key), e),
                        Err(_) => {}
                    }
                }
            })
            .await;
    });
    Ok(id)
}

/// Downloads remote files `prefix` + rel into local `root` + rel (Compare → "Download missing").
/// Relative paths come from the server, so they go through `safe_rel`; an existing local file is
/// never overwritten (it gets ` (n)`, ADR-006). Returns the job id.
#[tauri::command]
pub async fn download_rel(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    root: String,
    items: Vec<DlItem>,
) -> Result<String, String> {
    let c = state.remote(account.as_deref())?;
    let label = job_label(items.iter().map(|i| file_name(&i.key)).collect());
    let job = Job::new(&app, "down", label, c.account(), &bucket, &prefix);
    *job.local_dir.lock().unwrap() = Some(root.clone());
    *job.dest.lock().unwrap() = Some(root.clone());
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        let root = PathBuf::from(root);
        let files: Vec<(String, u64, PathBuf)> = items
            .into_iter()
            .map(|it| {
                let path = unique_path(root.join(safe_rel(&it.key)));
                (format!("{prefix}{}", it.key.trim_start_matches('/')), it.size, path)
            })
            .collect();
        j.files_total.store(files.len() as u64, Ordering::Relaxed);
        j.bytes_total.store(files.iter().map(|f| f.1).sum(), Ordering::Relaxed);
        stream::iter(files)
            .for_each_concurrent(FILE_CONCURRENCY, |(key, _, path)| {
                let (c, j, bucket) = (c.clone(), j.clone(), bucket.clone());
                async move {
                    if j.cancelled() {
                        return;
                    }
                    let down = || c.download_file(&bucket, &key, &path, &j);
                    match crate::resume::retry(j.resume, || j.cancelled(), down).await {
                        Ok(()) => {
                            j.files_done.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) if !j.cancelled() => j.fail(&file_name(&key), e),
                        Err(_) => {}
                    }
                }
            })
            .await;
    });
    Ok(id)
}

/// Copies one file between two accounts through a temp file under `tmp_root`, which keeps the
/// file's name: providers that guess the Content-Type from the file see a real one.
async fn relay(
    src: &Remote,
    (sb, key): (&str, &str),
    dst: &Remote,
    (db, dst_key): (&str, &str),
    size: u64,
    tmp_root: &Path,
    job: &Job,
) -> Result<(), String> {
    let tmp = tmp_root.join(uuid::Uuid::new_v4().to_string()).join(safe_rel(&file_name(key)));
    let r = async {
        src.download_file(sb, key, &tmp, job).await?;
        dst.upload_file(db, dst_key, &tmp, size, job).await
    }
    .await;
    let _ = tokio::fs::remove_file(&tmp).await;
    r
}

/// Copies remote files/folders from one view's account to another's (drag between remote views).
/// Same S3 account: server-side copy. Otherwise each file is relayed through a temp file (downloaded,
/// uploaded, deleted), so any provider pair works. Paths below `src_prefix` are kept under
/// `dst_prefix`. Returns the job id.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn copy_remote(
    app: AppHandle,
    state: State<'_, AppState>,
    src_account: Option<String>,
    src_bucket: String,
    src_prefix: String,
    items: Vec<DlItem>,
    dst_account: Option<String>,
    dst_bucket: String,
    dst_prefix: String,
) -> Result<String, String> {
    let src = state.remote(src_account.as_deref())?;
    let dst = state.remote(dst_account.as_deref())?;
    let same = src.account() == dst.account();
    let label = job_label(items.iter().map(|i| file_name(&i.key)).collect());
    let job = Job::new(&app, "copy", label, dst.account(), &dst_bucket, &dst_prefix);
    *job.src_dir.lock().unwrap() = Some(src_prefix.clone());
    *job.src.lock().unwrap() = Some((src.account().to_string(), src_bucket.clone()));
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        // Expand folders; keep each file's path relative to the source folder.
        let mut files: Vec<(String, u64, String)> = vec![];
        for it in items {
            let rel = it.key.strip_prefix(&src_prefix).unwrap_or(&it.key).to_string();
            if it.key.ends_with('/') {
                match src.list_all(&src_bucket, &it.key).await {
                    Ok(objs) => files.extend(objs.into_iter().filter(|(k, _)| !k.ends_with('/')).map(|(k, size)| {
                        let r = k.strip_prefix(&src_prefix).unwrap_or(&k).to_string();
                        (k, size, format!("{dst_prefix}{r}"))
                    })),
                    Err(e) => j.fail(&rel, e),
                }
            } else {
                files.push((it.key, it.size, format!("{dst_prefix}{rel}")));
            }
        }
        j.files_total.store(files.len() as u64, Ordering::Relaxed);
        // Relayed bytes count twice (down, then up); server-side copies once, at the end.
        let weight = if same { 1 } else { 2 };
        j.bytes_total.store(files.iter().map(|f| f.1 * weight).sum(), Ordering::Relaxed);
        let tmp_root = temp_root().join(format!("copy-{}", uuid::Uuid::new_v4()));
        stream::iter(files)
            .for_each_concurrent(FILE_CONCURRENCY, |(key, size, dst_key)| {
                let (src, dst, j, sb, db, tmp_root) =
                    (src.clone(), dst.clone(), j.clone(), src_bucket.clone(), dst_bucket.clone(), tmp_root.clone());
                async move {
                    if j.cancelled() {
                        return;
                    }
                    let res: Result<(), String> = async {
                        if same && src.copy_within((&sb, &key), (&db, &dst_key), size).await? {
                            j.flow(size).await;
                            return Ok(());
                        }
                        // Keep the file's name: providers that guess the type from the file see a real one.
                        relay(&src, (&sb, &key), &dst, (&db, &dst_key), size, &tmp_root, &j).await
                    }
                    .await;
                    match res {
                        Ok(()) => {
                            j.files_done.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) if !j.cancelled() => j.fail(&file_name(&key), e),
                        Err(_) => {}
                    }
                }
            })
            .await;
        let _ = tokio::fs::remove_dir_all(&tmp_root).await;
    });
    Ok(id)
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct DlItem {
    /// Object key; a key ending in `/` downloads the whole folder. For `download_rel`: the path
    /// relative to its `prefix`.
    key: String,
    size: u64,
}

fn default_download_dir(state: &AppState) -> PathBuf {
    match Some(state.settings().download_dir).filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d.trim()),
        None => dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default()),
    }
}

/// Starts a download job. Paths are relative to the current remote `prefix`, written under
/// `dest` (the left pane's folder). Top-level names that already exist get a ` (n)` suffix.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri commands take their arguments flat
pub async fn download(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    items: Vec<DlItem>,
    dest: Option<String>,
    resume: Option<bool>,
) -> Result<String, String> {
    let c = state.remote(account.as_deref())?;
    let dest = match dest.filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => default_download_dir(&state),
    };
    let label = job_label(items.iter().map(|i| file_name(&i.key)).collect());
    let job = Job::new(&app, "down", label.clone(), c.account(), &bucket, &prefix);
    if job.resume {
        crate::resume::pending_add(crate::resume::PendingJob {
            id: job.id.clone(),
            kind: "down".into(),
            label,
            account: account.clone(),
            bucket: bucket.clone(),
            prefix: prefix.clone(),
            paths: vec![],
            items: items.clone(),
            dest: Some(dest.to_string_lossy().into_owned()),
        });
    }
    let resuming = resume.unwrap_or(false);
    // Where a file goes. Normally never next to an existing name (`unique_path`). When resuming, a
    // file already there with the right size is done (None), and a `.part` next to it continues.
    let target = move |p: PathBuf, size: u64| -> Option<PathBuf> {
        if !resuming {
            return Some(unique_path(p));
        }
        match std::fs::metadata(&p) {
            Ok(m) if m.len() == size => None,
            Ok(_) => Some(unique_path(p)),
            Err(_) => Some(p),
        }
    };
    *job.local_dir.lock().unwrap() = Some(dest.to_string_lossy().to_string());
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        // Resolve folders into files; map each to a local path under `dest`.
        let mut files: Vec<(String, u64, PathBuf)> = vec![];
        let mut skipped: Vec<u64> = vec![]; // resuming: files already complete
        let mut reveal: Option<PathBuf> = None;
        for it in items {
            let rel = it.key.strip_prefix(&prefix).unwrap_or(&it.key).to_string();
            if it.key.ends_with('/') {
                // Resuming continues in the same folder; otherwise a new one never mixes with old files.
                let root = if resuming { dest.join(safe_rel(&rel)) } else { unique_path(dest.join(safe_rel(&rel))) };
                reveal.get_or_insert(root.clone());
                match c.list_all(&bucket, &it.key).await {
                    Ok(objs) => {
                        for (k, size) in objs.into_iter().filter(|(k, _)| !k.ends_with('/')) {
                            let sub = safe_rel(&k[it.key.len()..]);
                            match target(root.join(sub), size) {
                                Some(p) => files.push((k, size, p)),
                                None => skipped.push(size),
                            }
                        }
                    }
                    Err(e) => j.fail(&rel, e),
                }
            } else {
                let p = dest.join(safe_rel(&file_name(&rel)));
                match target(p.clone(), it.size) {
                    Some(p) => {
                        reveal.get_or_insert(p.clone());
                        files.push((it.key, it.size, p));
                    }
                    None => {
                        reveal.get_or_insert(p);
                        skipped.push(it.size);
                    }
                }
            }
        }
        *j.dest.lock().unwrap() = reveal.map(|p| p.to_string_lossy().to_string());
        j.files_total.store((files.len() + skipped.len()) as u64, Ordering::Relaxed);
        j.bytes_total.store(files.iter().map(|f| f.1).chain(skipped.iter().copied()).sum(), Ordering::Relaxed);
        j.files_done.store(skipped.len() as u64, Ordering::Relaxed);
        j.bytes_done.store(skipped.iter().sum(), Ordering::Relaxed);
        stream::iter(files)
            .for_each_concurrent(FILE_CONCURRENCY, |(key, _, path)| {
                let (c, j, bucket) = (c.clone(), j.clone(), bucket.clone());
                async move {
                    if j.cancelled() {
                        return;
                    }
                    match c.download_file(&bucket, &key, &path, &j).await {
                        Ok(()) => {
                            j.files_done.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) if !j.cancelled() => j.fail(&file_name(&key), e),
                        Err(_) => {}
                    }
                }
            })
            .await;
    });
    Ok(id)
}

/// Downloads an object to a temp folder and opens it with the system default app.
#[tauri::command]
pub async fn open_object(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    key: String,
) -> Result<(), String> {
    let c = state.remote(account.as_deref())?;
    let path = temp_root().join(uuid::Uuid::new_v4().to_string()).join(safe_rel(&file_name(&key)));
    let job = Job::untracked("open", file_name(&key), c.account(), &bucket, "");
    c.download_file(&bucket, &key, &path, &job).await?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Profile, check_profile};
    use crate::provider::Remote;
    use crate::s3::make_client;

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("noexplorer-ut-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn collect_uploads_keeps_structure_and_skips_junk() {
        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("proj/src")).unwrap();
        std::fs::write(dir.join("proj/src/main.rs"), "fn main(){}").unwrap();
        std::fs::write(dir.join("proj/.DS_Store"), "x").unwrap();
        std::fs::write(dir.join("note.txt"), "hi").unwrap();
        let mut files = collect_uploads(
            &[dir.join("proj").to_string_lossy().into(), dir.join("note.txt").to_string_lossy().into()],
            "backup/",
        );
        files.sort_by(|a, b| a.1.cmp(&b.1));
        let keys: Vec<&str> = files.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(keys, ["backup/note.txt", "backup/proj/src/main.rs"]);
        assert_eq!(files[0].2, 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// End-to-end against a real S3 server. Skipped unless NOEXPLORER_TEST_ENDPOINT is set:
    /// `./scripts/dev-s3.sh up` then
    /// `NOEXPLORER_TEST_ENDPOINT=http://localhost:8333 NOEXPLORER_TEST_KEY=devkey NOEXPLORER_TEST_SECRET=devsecret123 cargo test -- --nocapture`
    #[tokio::test]
    async fn s3_roundtrip() {
        let Ok(endpoint) = std::env::var("NOEXPLORER_TEST_ENDPOINT") else {
            eprintln!("skipping s3_roundtrip: NOEXPLORER_TEST_ENDPOINT not set");
            return;
        };
        let cfg = Profile {
            endpoint,
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            ..Default::default()
        };
        let bucket = "noexplorer-test";
        let _ = make_client(&cfg).create_bucket().bucket(bucket).send().await;
        assert!(check_profile(&cfg).await.unwrap().contains(&bucket.to_string()));
        let c = Remote::new(&cfg);

        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("folder/sub")).unwrap();
        let big: Vec<u8> = (0..(40 * 1024 * 1024 + 123)).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(dir.join("folder/big.bin"), &big).unwrap();
        std::fs::write(dir.join("folder/sub/hello.txt"), "xin chào").unwrap();

        let files = collect_uploads(&[dir.join("folder").to_string_lossy().to_string()], "rt/");
        let job = Job::untracked("up", "t".into(), "", bucket, "");
        for (p, k, s) in &files {
            c.upload_file(bucket, k, p, *s, &job).await.unwrap(); // big.bin goes multipart
        }
        let all = c.list_all(bucket, "rt/").await.unwrap();
        assert!(all.iter().any(|(k, s)| k == "rt/folder/big.bin" && *s == big.len() as u64));

        let out = dir.join("out/big.bin");
        c.download_file(bucket, "rt/folder/big.bin", &out, &job).await.unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), big);

        let url = c.link(bucket, "rt/folder/sub/hello.txt", 60, false).await.unwrap();
        assert!(url.contains("/noexplorer-test/rt/folder/sub/hello.txt"), "path-style expected");
        assert_eq!(c.read_head(bucket, "rt/folder/sub/hello.txt", 3).await.unwrap(), b"xin");
        // Server-side copy, with a key that needs encoding.
        assert!(
            c.copy_within((bucket, "rt/folder/sub/hello.txt"), (bucket, "rt/copy/xin chào #1.txt"), 9).await.unwrap()
        );
        assert_eq!(c.read_head(bucket, "rt/copy/xin chào #1.txt", 3).await.unwrap(), b"xin");
        let listed = c.list(bucket, "rt/folder/").await.unwrap();
        assert!(listed.iter().any(|e| e.dir && e.key == "rt/folder/sub/"));

        // Rename, duplicate a folder, and never overwrite (file ops, experimental).
        c.transfer_within(bucket, "rt/copy/xin chào #1.txt", "rt/copy/renamed.txt", false, None).await.unwrap();
        let copy_dir = c.list_all(bucket, "rt/copy/").await.unwrap();
        assert!(copy_dir.iter().any(|(k, _)| k == "rt/copy/renamed.txt"));
        assert!(!copy_dir.iter().any(|(k, _)| k == "rt/copy/xin chào #1.txt"), "move removes the source");
        c.transfer_within(bucket, "rt/folder/sub/", "rt/dup/", true, None).await.unwrap();
        assert_eq!(c.read_head(bucket, "rt/dup/hello.txt", 3).await.unwrap(), b"xin");
        assert!(c.transfer_within(bucket, "rt/folder/sub/", "rt/dup/", true, None).await.is_err(), "target exists");
        assert!(c.transfer_within(bucket, "rt/folder/", "rt/folder/sub/x/", false, None).await.is_err());

        let before = c.list_all(bucket, "rt/").await.unwrap().len();
        assert_eq!(c.delete(bucket, vec!["rt/".into()], None).await.unwrap(), before + 1); // + the "rt/" key
        assert!(c.list_all(bucket, "rt/").await.unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Copy between two accounts (two clients, two buckets on dev S3; see `s3_roundtrip` for the env
    /// vars): bytes arrive intact, the Content-Type comes from the name, and no temp file is left.
    #[tokio::test]
    async fn relay_copy_between_accounts() {
        let Ok(endpoint) = std::env::var("NOEXPLORER_TEST_ENDPOINT") else {
            eprintln!("skipping relay_copy_between_accounts: NOEXPLORER_TEST_ENDPOINT not set");
            return;
        };
        let cfg = Profile {
            endpoint,
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            ..Default::default()
        };
        let s3 = make_client(&cfg);
        let (from, to) = ("noexplorer-test", "noexplorer-test-2");
        let _ = s3.create_bucket().bucket(from).send().await;
        let _ = s3.create_bucket().bucket(to).send().await;
        let (a, b) = (Remote::new(&cfg), Remote::new(&cfg)); // two accounts: no server-side copy
        let dir = tmp_dir();
        let data: Vec<u8> = (0..(9 * 1024 * 1024 + 5)).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(dir.join("src.bin"), &data).unwrap();
        let job = Job::untracked("copy", "t".into(), "", to, "");
        a.upload_file(from, "rl/report final.pdf", &dir.join("src.bin"), data.len() as u64, &job).await.unwrap();

        let tmp_root = dir.join("relay");
        let size = data.len() as u64;
        relay(&a, (from, "rl/report final.pdf"), &b, (to, "in/report final.pdf"), size, &tmp_root, &job).await.unwrap();

        let head = s3.head_object().bucket(to).key("in/report final.pdf").send().await.unwrap();
        assert_eq!(head.content_type(), Some("application/pdf"));
        let out = dir.join("back.pdf");
        b.download_file(to, "in/report final.pdf", &out, &job).await.unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), data);
        let left =
            walkdir::WalkDir::new(&tmp_root).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file());
        assert_eq!(left.count(), 0, "temp files left behind");

        a.delete(from, vec!["rl/".into()], None).await.unwrap();
        b.delete(to, vec!["in/".into()], None).await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Resumable transfers against dev S3 (see `s3_roundtrip` for the env vars): a multipart upload
    /// continues from the parts already on the server, and a download continues its `.part`.
    #[tokio::test]
    async fn s3_resume() {
        let Ok(endpoint) = std::env::var("NOEXPLORER_TEST_ENDPOINT") else {
            eprintln!("skipping s3_resume: NOEXPLORER_TEST_ENDPOINT not set");
            return;
        };
        let cfg = Profile {
            endpoint,
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            ..Default::default()
        };
        let bucket = "noexplorer-test";
        let s3 = make_client(&cfg);
        let _ = s3.create_bucket().bucket(bucket).send().await;
        let c = Remote::new(&cfg);
        let dir = tmp_dir();
        crate::resume::init_dir(dir.join("cfg"));

        // 20 MB → three 8 MB parts. The "interrupted" upload already sent part 1 and was journaled.
        let data: Vec<u8> = (0..(20 * 1024 * 1024 + 7)).map(|i| (i * 13 % 251) as u8).collect();
        let f = dir.join("big.bin");
        std::fs::write(&f, &data).unwrap();
        let key = "rs/big.bin";
        let up = s3.create_multipart_upload().bucket(bucket).key(key).send().await.unwrap();
        let id = up.upload_id().unwrap().to_string();
        s3.upload_part()
            .bucket(bucket)
            .key(key)
            .upload_id(&id)
            .part_number(1)
            .body(aws_sdk_s3::primitives::ByteStream::from(data[..8 * 1024 * 1024].to_vec()))
            .send()
            .await
            .unwrap();
        crate::resume::multipart_put(c.account(), bucket, key, data.len() as u64, &f, &id);

        let mut job = Job::untracked("up", "t".into(), "", bucket, "");
        job.resume = true;
        c.upload_file(bucket, key, &f, data.len() as u64, &job).await.unwrap();
        assert!(crate::resume::multipart_find(c.account(), bucket, key, data.len() as u64, &f).is_none());

        // A download that stopped after 5 MB continues from its .part.
        let out = dir.join("out/big.bin");
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::write(dir.join("out/big.bin.part"), &data[..5 * 1024 * 1024]).unwrap();
        c.download_file(bucket, key, &out, &job).await.unwrap();
        assert!(std::fs::read(&out).unwrap() == data, "resumed download differs");
        assert!(!dir.join("out/big.bin.part").exists());

        c.delete(bucket, vec!["rs/".into()], None).await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Pause, speed limit and the interrupted-transfer journal, against dev S3.
    #[tokio::test]
    async fn s3_pause_limit_journal() {
        let Ok(endpoint) = std::env::var("NOEXPLORER_TEST_ENDPOINT") else {
            eprintln!("skipping s3_pause_limit_journal: NOEXPLORER_TEST_ENDPOINT not set");
            return;
        };
        let cfg = Profile {
            endpoint,
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            ..Default::default()
        };
        let bucket = "noexplorer-test";
        let _ = make_client(&cfg).create_bucket().bucket(bucket).send().await;
        let c = std::sync::Arc::new(Remote::new(&cfg));
        let dir = tmp_dir();
        let mb: Vec<u8> = vec![7u8; 1024 * 1024];
        for i in 0..3 {
            std::fs::write(dir.join(format!("f{i}.bin")), &mb).unwrap();
        }

        // Speed limit: 3 × 1 MB at 1 MB/s takes about 2-3 s (the first window is free). Only jobs
        // started with the experiment on are limited.
        let turn = crate::resume::LIMIT_TEST.lock().await;
        crate::resume::set_speed_limit(1024);
        let mut j = Job::untracked("up", "t".into(), "", bucket, "");
        j.resume = true;
        let job = std::sync::Arc::new(j);
        let t = std::time::Instant::now();
        futures::future::join_all((0..3).map(|i| {
            let (c, job, p) = (c.clone(), job.clone(), dir.join(format!("f{i}.bin")));
            async move { c.upload_file(bucket, &format!("pl/f{i}.bin"), &p, 1024 * 1024, &job).await.unwrap() }
        }))
        .await;
        let took = t.elapsed().as_secs_f64();
        crate::resume::set_speed_limit(0);
        drop(turn);
        assert!((1.8..6.0).contains(&took), "3 MB at 1 MB/s took {took}s");

        // Pause: nothing moves while paused, and the upload finishes after resuming.
        let job = std::sync::Arc::new(Job::untracked("up", "t".into(), "", bucket, ""));
        job.set_paused(true);
        let task = {
            let (c, job, p) = (c.clone(), job.clone(), dir.join("f0.bin"));
            tokio::spawn(async move { c.upload_file(bucket, "pl/paused.bin", &p, 1024 * 1024, &job).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        assert_eq!(job.bytes_done.load(Ordering::Relaxed), 0, "moved while paused");
        assert!(!task.is_finished());
        job.set_paused(false);
        task.await.unwrap().unwrap();
        assert_eq!(job.bytes_done.load(Ordering::Relaxed), 1024 * 1024);

        // Journal: an entry stays until removed (the job's end removes it unless it failed).
        crate::resume::init_dir(dir.join("cfg"));
        crate::resume::pending_add(crate::resume::PendingJob {
            id: "j1".into(),
            kind: "up".into(),
            label: "f0.bin".into(),
            account: None,
            bucket: bucket.into(),
            prefix: "pl/".into(),
            paths: vec![dir.join("f0.bin").to_string_lossy().into_owned()],
            items: vec![],
            dest: None,
        });
        assert_eq!(crate::resume::pending_list().len(), 1);
        crate::resume::pending_remove("j1");
        assert!(crate::resume::pending_list().is_empty());

        c.delete(bucket, vec!["pl/".into()], None).await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
