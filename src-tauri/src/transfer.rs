//! Upload / download / open jobs, provider-agnostic (ADR-025): each job holds its account's
//! `Remote` and moves up to [`FILE_CONCURRENCY`] files at once (ADR-005). How a single file moves
//! (multipart, chunking, `.part` files) is the provider's business, see `s3.rs`.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use futures::{StreamExt, stream};
use serde::Deserialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::jobs::Job;
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
) -> Result<String, String> {
    let c = state.remote(account.as_deref())?;
    let label = job_label(paths.iter().map(|p| file_name(&p.replace('\\', "/"))).collect());
    let job = Job::new(&app, "up", label, c.account(), &bucket, &prefix);
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        let files = tokio::task::spawn_blocking(move || collect_uploads(&paths, &prefix)).await.unwrap_or_default();
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

#[derive(Deserialize)]
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
pub async fn download(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    items: Vec<DlItem>,
    dest: Option<String>,
) -> Result<String, String> {
    let c = state.remote(account.as_deref())?;
    let dest = match dest.filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => default_download_dir(&state),
    };
    let label = job_label(items.iter().map(|i| file_name(&i.key)).collect());
    let job = Job::new(&app, "down", label, c.account(), &bucket, &prefix);
    *job.local_dir.lock().unwrap() = Some(dest.to_string_lossy().to_string());
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        // Resolve folders into files; map each to a local path under `dest`.
        let mut files: Vec<(String, u64, PathBuf)> = vec![];
        let mut reveal: Option<PathBuf> = None;
        for it in items {
            let rel = it.key.strip_prefix(&prefix).unwrap_or(&it.key).to_string();
            if it.key.ends_with('/') {
                let root = unique_path(dest.join(safe_rel(&rel)));
                reveal.get_or_insert(root.clone());
                match c.list_all(&bucket, &it.key).await {
                    Ok(objs) => {
                        for (k, size) in objs.into_iter().filter(|(k, _)| !k.ends_with('/')) {
                            let sub = safe_rel(&k[it.key.len()..]);
                            files.push((k, size, root.join(sub)));
                        }
                    }
                    Err(e) => j.fail(&rel, e),
                }
            } else {
                let p = unique_path(dest.join(safe_rel(&file_name(&rel))));
                reveal.get_or_insert(p.clone());
                files.push((it.key, it.size, p));
            }
        }
        *j.dest.lock().unwrap() = reveal.map(|p| p.to_string_lossy().to_string());
        j.files_total.store(files.len() as u64, Ordering::Relaxed);
        j.bytes_total.store(files.iter().map(|f| f.1).sum(), Ordering::Relaxed);
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
        let listed = c.list(bucket, "rt/folder/").await.unwrap();
        assert!(listed.iter().any(|e| e.dir && e.key == "rt/folder/sub/"));

        assert_eq!(c.delete(bucket, vec!["rt/".into()]).await.unwrap(), all.len() + 1);
        assert!(c.list_all(bucket, "rt/").await.unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
