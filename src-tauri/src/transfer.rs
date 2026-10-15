//! Upload / download / open engine.
//!
//! Throughput knobs (see ADR-005): up to [`FILE_CONCURRENCY`] files in flight per job; files
//! above [`MULTIPART_THRESHOLD`] use multipart upload with [`PART_CONCURRENCY`] parts in flight.
//! Downloads stream to `<name>.part` and are renamed on success, so partial files never look complete.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use futures::{StreamExt, TryStreamExt, stream};
use serde::Deserialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::jobs::Job;
use crate::remote::list_all;
use crate::state::AppState;
use crate::util::{err, file_name, job_label, safe_rel, unique_path};

pub const MULTIPART_THRESHOLD: u64 = 16 * 1024 * 1024;
pub const MIN_PART_SIZE: u64 = 8 * 1024 * 1024;
/// S3 allows 10 000 parts; stay well below so huge files still fit.
const MAX_PARTS: u64 = 9_000;
pub const PART_CONCURRENCY: usize = 4;
pub const FILE_CONCURRENCY: usize = 6;

/// Junk files never uploaded from folders.
const SKIP_NAMES: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

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
    bucket: String,
    prefix: String,
    paths: Vec<String>,
) -> Result<String, String> {
    let c = state.client()?;
    let label = job_label(paths.iter().map(|p| file_name(&p.replace('\\', "/"))).collect());
    let job = Job::new(&app, "up", label, &bucket, &prefix);
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
                    match upload_file(&c, &bucket, &key, &path, size, &j).await {
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

/// Single PUT for small files, parallel multipart for large ones (aborted on failure).
pub async fn upload_file(c: &Client, bucket: &str, key: &str, path: &Path, size: u64, job: &Job) -> Result<(), String> {
    let ct = mime_guess::from_path(path).first_or_octet_stream().to_string();
    if size <= MULTIPART_THRESHOLD {
        let body = ByteStream::from_path(path).await.map_err(err)?;
        c.put_object().bucket(bucket).key(key).content_type(ct).body(body).send().await.map_err(err)?;
        job.bytes_done.fetch_add(size, Ordering::Relaxed);
        return Ok(());
    }

    let part_size = MIN_PART_SIZE.max(size.div_ceil(MAX_PARTS));
    let parts = size.div_ceil(part_size);
    let created = c.create_multipart_upload().bucket(bucket).key(key).content_type(ct).send().await.map_err(err)?;
    let upload_id = created.upload_id().ok_or("Server returned no upload id")?.to_string();

    let result: Result<(), String> = async {
        let mut done: Vec<CompletedPart> = stream::iter(0..parts)
            .map(|i| {
                let upload_id = upload_id.clone();
                async move {
                    if job.cancelled() {
                        return Err("cancelled".to_string());
                    }
                    let offset = i * part_size;
                    let len = part_size.min(size - offset);
                    let mut f = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
                    f.seek(SeekFrom::Start(offset)).await.map_err(|e| e.to_string())?;
                    let mut buf = vec![0u8; len as usize];
                    f.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
                    let n = (i + 1) as i32;
                    let r = c
                        .upload_part()
                        .bucket(bucket)
                        .key(key)
                        .upload_id(&upload_id)
                        .part_number(n)
                        .body(ByteStream::from(buf))
                        .send()
                        .await
                        .map_err(err)?;
                    job.bytes_done.fetch_add(len, Ordering::Relaxed);
                    Ok(CompletedPart::builder().part_number(n).set_e_tag(r.e_tag().map(String::from)).build())
                }
            })
            .buffer_unordered(PART_CONCURRENCY)
            .try_collect()
            .await?;
        done.sort_by_key(|p| p.part_number());
        c.complete_multipart_upload()
            .bucket(bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(done)).build())
            .send()
            .await
            .map_err(err)?;
        Ok(())
    }
    .await;

    if result.is_err() {
        let _ = c.abort_multipart_upload().bucket(bucket).key(key).upload_id(&upload_id).send().await;
    }
    result
}

#[derive(Deserialize)]
pub struct DlItem {
    /// Object key; a key ending in `/` downloads the whole folder.
    key: String,
    size: u64,
}

fn default_download_dir(state: &AppState) -> PathBuf {
    match state.config().map(|c| c.download_dir).filter(|d| !d.trim().is_empty()) {
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
    bucket: String,
    prefix: String,
    items: Vec<DlItem>,
    dest: Option<String>,
) -> Result<String, String> {
    let c = state.client()?;
    let dest = match dest.filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => default_download_dir(&state),
    };
    let label = job_label(items.iter().map(|i| file_name(&i.key)).collect());
    let job = Job::new(&app, "down", label, &bucket, &prefix);
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
                match list_all(&c, &bucket, &it.key).await {
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
                    match download_file(&c, &bucket, &key, &path, &j).await {
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

/// Streams one object to `path` via `path.part`, renaming on success.
pub async fn download_file(c: &Client, bucket: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let r = c.get_object().bucket(bucket).key(key).send().await.map_err(err)?;
    let mut body = r.body;
    let f = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
    let mut w = tokio::io::BufWriter::with_capacity(1 << 20, f);
    let res: Result<(), String> = async {
        while let Some(chunk) = body.try_next().await.map_err(err)? {
            if job.cancelled() {
                return Err("cancelled".into());
            }
            w.write_all(&chunk).await.map_err(|e| e.to_string())?;
            job.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        w.flush().await.map_err(|e| e.to_string())
    }
    .await;
    drop(w);
    match res {
        Ok(()) => tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
    }
}

/// Downloads an object to a temp folder and opens it with the system default app.
#[tauri::command]
pub async fn open_object(
    app: AppHandle,
    state: State<'_, AppState>,
    bucket: String,
    key: String,
) -> Result<(), String> {
    let c = state.client()?;
    let path = temp_root().join(uuid::Uuid::new_v4().to_string()).join(safe_rel(&file_name(&key)));
    let job = Job::untracked("open", file_name(&key), &bucket, "");
    download_file(&c, &bucket, &key, &path, &job).await?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, make_client, test_config};
    use aws_sdk_s3::presigning::PresigningConfig;
    use std::time::Duration;

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
        let cfg = Config {
            endpoint,
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            ..Default::default()
        };
        let c = make_client(&cfg);
        let bucket = "noexplorer-test";
        let _ = c.create_bucket().bucket(bucket).send().await;
        assert!(test_config(cfg.clone()).await.unwrap().contains(&bucket.to_string()));

        let dir = tmp_dir();
        std::fs::create_dir_all(dir.join("folder/sub")).unwrap();
        let big: Vec<u8> = (0..(40 * 1024 * 1024 + 123)).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(dir.join("folder/big.bin"), &big).unwrap();
        std::fs::write(dir.join("folder/sub/hello.txt"), "xin chào").unwrap();

        let files = collect_uploads(&[dir.join("folder").to_string_lossy().to_string()], "rt/");
        let job = Job::untracked("up", "t".into(), bucket, "");
        for (p, k, s) in &files {
            upload_file(&c, bucket, k, p, *s, &job).await.unwrap(); // big.bin goes multipart
        }
        let all = list_all(&c, bucket, "rt/").await.unwrap();
        assert!(all.iter().any(|(k, s)| k == "rt/folder/big.bin" && *s == big.len() as u64));

        let out = dir.join("out/big.bin");
        download_file(&c, bucket, "rt/folder/big.bin", &out, &job).await.unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), big);

        let url = c
            .get_object()
            .bucket(bucket)
            .key("rt/folder/sub/hello.txt")
            .presigned(PresigningConfig::expires_in(Duration::from_secs(60)).unwrap())
            .await
            .unwrap();
        assert!(url.uri().contains("/noexplorer-test/rt/folder/sub/hello.txt"), "path-style expected");

        for (k, _) in all {
            c.delete_object().bucket(bucket).key(k).send().await.unwrap();
        }
        assert!(list_all(&c, bucket, "rt/").await.unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
