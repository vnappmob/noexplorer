//! Speed test between two places (experimental, ADR-050): this computer ↔ a remote, or remote A ↔
//! remote B. It moves generated test data the way real transfers do (the same provider code, files
//! in parallel per provider, multipart / chunked uploads), so the numbers compare services as
//! NoExplorer actually uses them.
//!
//! - Tests: one large file (throughput) and many small files (per-file overhead), plus the
//!   response time of a listing on each remote.
//! - Data is random (incompressible), written to the OS temp folder, and to
//!   `<folder>/.noexplorer-speedtest-<id>/` on each remote; everything is deleted at the end, also
//!   after a failure or Cancel, and the deletion is checked. Test folders an earlier run left
//!   behind (the app quit mid-run) are removed before the next run in the same folder. What
//!   happened goes into `notes`.
//! - A remote → remote run goes the way a copy between them goes: server-side within one account
//!   when the provider can, otherwise relayed through this computer (download, then upload).
//! - Progress and results stream as `"speedtest"` events; `cancel(id)` stops it.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::jobs::Job;
use crate::provider::Remote;
use crate::state::AppState;

/// One end of the test.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    /// Shown in results ("This computer", "Home MinIO · media").
    pub name: String,
    /// None = this computer.
    pub account: Option<String>,
    #[serde(default)]
    pub bucket: String,
    /// Folder the test folder goes in (remote), "" = the bucket's top.
    #[serde(default)]
    pub prefix: String,
}

/// What to move.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub big_mb: u64,
    pub small_count: u64,
    pub small_kb: u64,
    /// Also measure the way back (B → A).
    pub both: bool,
}

/// One measured result. Mirrors `SpeedResult` in `src/types.ts`.
#[derive(Serialize, Clone)]
pub struct Measure {
    /// "A → B", or the remote's name for response times.
    pub route: String,
    /// "large" | "small" | "latency"
    pub test: &'static str,
    pub bytes: u64,
    pub files: u64,
    /// Elapsed seconds (latency: the median of a few listings).
    pub secs: f64,
}

/// Payload of the `"speedtest"` event. Mirrors `SpeedEvent` in `src/types.ts`.
#[derive(Serialize, Clone)]
struct SpeedEvent {
    id: String,
    /// What's happening, for the progress line.
    phase: String,
    done: u64,
    total: u64,
    results: Vec<Measure>,
    /// Cleanup and other remarks for the user ("Removed 2 old test folders from …").
    notes: Vec<String>,
    finished: bool,
    error: Option<String>,
}

/// A remote end, resolved.
struct End {
    name: String,
    remote: Arc<Remote>,
    bucket: String,
    /// `<prefix>.noexplorer-speedtest-<id>/`
    dir: String,
}

/// A generated file: path in the temp folder, key relative to a test folder, size.
type File = (PathBuf, String, u64);

struct Run {
    /// Sends a progress event (to the window; tests collect them).
    emit: Box<dyn Fn(SpeedEvent) + Send + Sync>,
    id: String,
    job: Arc<Job>,
    phase: Mutex<String>,
    total: std::sync::atomic::AtomicU64,
    results: Mutex<Vec<Measure>>,
    notes: Mutex<Vec<String>>,
    tmp: PathBuf,
}

impl Run {
    fn event(&self, finished: bool, error: Option<String>) {
        (self.emit)(SpeedEvent {
            id: self.id.clone(),
            phase: self.phase.lock().unwrap().clone(),
            done: self.job.bytes_done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            results: self.results.lock().unwrap().clone(),
            notes: self.notes.lock().unwrap().clone(),
            finished,
            error,
        });
    }

    /// Starts a step: its label and how many bytes it moves (progress restarts at 0).
    fn step(&self, phase: String, bytes: u64) {
        *self.phase.lock().unwrap() = phase;
        self.total.store(bytes, Ordering::Relaxed);
        self.job.bytes_done.store(0, Ordering::Relaxed);
        self.event(false, None);
    }

    fn check(&self) -> Result<(), String> {
        if self.job.cancelled() { Err("cancelled".into()) } else { Ok(()) }
    }
}

/// Fills `buf` with fast pseudo-random bytes (xorshift): incompressible, so no provider or proxy
/// can shrink it on the way.
fn random_fill(buf: &mut [u8], seed: &mut u64) {
    for chunk in buf.chunks_mut(8) {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        chunk.copy_from_slice(&seed.to_le_bytes()[..chunk.len()]);
    }
}

/// Writes the test files: `large.bin` and `small/NNNN.bin`. Returns (large, small files).
fn generate(dir: &Path, plan: &Plan) -> std::io::Result<(Vec<File>, Vec<File>)> {
    use std::io::Write;
    std::fs::create_dir_all(dir.join("small"))?;
    let mut seed = 0x9E37_79B9_7F4A_7C15u64 ^ std::process::id() as u64;
    let mut buf = vec![0u8; 1 << 20];
    let big = plan.big_mb.clamp(1, 1024) << 20;
    let mut f = std::io::BufWriter::new(std::fs::File::create(dir.join("large.bin"))?);
    let mut left = big;
    while left > 0 {
        let n = left.min(buf.len() as u64) as usize;
        random_fill(&mut buf[..n], &mut seed);
        f.write_all(&buf[..n])?;
        left -= n as u64;
    }
    f.flush()?;
    let large = vec![(dir.join("large.bin"), "large.bin".to_string(), big)];
    let size = plan.small_kb.clamp(1, 64 * 1024) << 10;
    let mut small = vec![];
    for i in 0..plan.small_count.clamp(1, 5000) {
        let mut b = vec![0u8; size as usize];
        random_fill(&mut b, &mut seed);
        let name = format!("small/{i:04}.bin");
        std::fs::write(dir.join(&name), &b)?;
        small.push((dir.join(&name), name, size));
    }
    Ok((large, small))
}

/// Uploads `files` into `end.dir + sub`, as many at once as the provider moves in real transfers.
async fn put(run: &Run, end: &End, sub: &str, files: &[File]) -> Result<(), String> {
    let n = end.remote.file_concurrency();
    let res: Vec<Result<(), String>> = futures::stream::iter(files.to_vec())
        .map(|(path, key, size)| async move {
            run.check()?;
            let key = format!("{}{sub}{key}", end.dir);
            end.remote.upload_file(&end.bucket, &key, &path, size, &run.job).await
        })
        .buffer_unordered(n)
        .collect()
        .await;
    res.into_iter().collect()
}

/// Downloads `files` from `end.dir + sub` into `to`.
async fn get(run: &Run, end: &End, sub: &str, files: &[File], to: &Path) -> Result<(), String> {
    let n = end.remote.file_concurrency();
    let res: Vec<Result<(), String>> = futures::stream::iter(files.to_vec())
        .map(|(_, key, _)| async move {
            run.check()?;
            let dest = to.join(&key);
            let _ = tokio::fs::remove_file(&dest).await; // a previous direction's copy
            end.remote.download_file(&end.bucket, &format!("{}{sub}{key}", end.dir), &dest, &run.job).await
        })
        .buffer_unordered(n)
        .collect()
        .await;
    res.into_iter().collect()
}

/// Copies `files` from `a.dir + from` to `b.dir + to` like a copy between accounts: on the server
/// when both are one account that can, else relayed through a temp file.
async fn copy(run: &Run, a: &End, from: &str, b: &End, to: &str, files: &[File]) -> Result<(), String> {
    let same = Arc::ptr_eq(&a.remote, &b.remote);
    let n = a.remote.file_concurrency().min(b.remote.file_concurrency());
    let relay = run.tmp.join(format!("relay-{}", uuid::Uuid::new_v4().simple()));
    tokio::fs::create_dir_all(relay.join("small")).await.map_err(|e| e.to_string())?;
    let res: Vec<Result<(), String>> = futures::stream::iter(files.to_vec())
        .map(|(_, key, size)| {
            let relay = relay.clone();
            async move {
                run.check()?;
                let (src, dst) = (format!("{}{from}{key}", a.dir), format!("{}{to}{key}", b.dir));
                if same && a.remote.copy_within((&a.bucket, &src), (&b.bucket, &dst), size).await? {
                    run.job.bytes_done.fetch_add(size * 2, Ordering::Relaxed); // progress counts down + up
                    return Ok(());
                }
                let tmp = relay.join(&key);
                a.remote.download_file(&a.bucket, &src, &tmp, &run.job).await?;
                let up = b.remote.upload_file(&b.bucket, &dst, &tmp, size, &run.job).await;
                let _ = tokio::fs::remove_file(&tmp).await;
                up
            }
        })
        .buffer_unordered(n)
        .collect()
        .await;
    let _ = tokio::fs::remove_dir_all(&relay).await;
    res.into_iter().collect()
}

/// Median time of three listings of the folder the test runs in.
async fn latency(run: &Run, end: &End) -> Result<(), String> {
    let parent = parent_of(&end.dir);
    let mut t = vec![];
    for _ in 0..3 {
        run.check()?;
        let start = Instant::now();
        end.remote.list(&end.bucket, &parent).await?;
        t.push(start.elapsed().as_secs_f64());
    }
    t.sort_by(f64::total_cmp);
    run.results.lock().unwrap().push(Measure {
        route: end.name.clone(),
        test: "latency",
        bytes: 0,
        files: 3,
        secs: t[1],
    });
    Ok(())
}

/// Runs one measured step and records it.
async fn measure<F: std::future::Future<Output = Result<(), String>>>(
    run: &Run,
    route: &str,
    test: &'static str,
    files: &[File],
    weight: u64,
    f: F,
) -> Result<(), String> {
    let bytes: u64 = files.iter().map(|f| f.2).sum();
    let what = if test == "large" { "One large file" } else { "Many small files" };
    run.step(format!("{what}: {route}"), bytes * weight);
    let start = Instant::now();
    f.await?;
    let secs = start.elapsed().as_secs_f64();
    run.results.lock().unwrap().push(Measure { route: route.into(), test, bytes, files: files.len() as u64, secs });
    run.event(false, None);
    Ok(())
}

async fn run_test(
    run: &Run,
    a: &Node,
    b: &Node,
    ea: Option<&End>,
    eb: Option<&End>,
    plan: &Plan,
) -> Result<(), String> {
    run.step("Preparing test data…".into(), 0);
    let data = run.tmp.join("data");
    let plan2 = *plan;
    let d2 = data.clone();
    let (large, small) = tokio::task::spawn_blocking(move || generate(&d2, &plan2))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Couldn't write test data: {e}"))?;
    let down = run.tmp.join("down");
    tokio::fs::create_dir_all(down.join("small")).await.map_err(|e| format!("Couldn't write test data: {e}"))?;
    for e in [ea, eb].into_iter().flatten() {
        run.step(format!("Response time: {}", e.name), 0);
        latency(run, e).await?;
    }
    let ab = format!("{} → {}", a.name, b.name);
    let ba = format!("{} → {}", b.name, a.name);
    for (test, files) in [("large", &large), ("small", &small)] {
        match (ea, eb) {
            // This computer ↔ remote: upload, then download what was uploaded.
            (None, Some(eb)) => {
                measure(run, &ab, test, files, 1, put(run, eb, "1/", files)).await?;
                if plan.both {
                    measure(run, &ba, test, files, 1, get(run, eb, "1/", files, &down)).await?;
                }
            }
            (Some(ea), None) => {
                run.step(format!("Putting test data on {}…", ea.name), files.iter().map(|f| f.2).sum());
                put(run, ea, "1/", files).await?;
                measure(run, &ab, test, files, 1, get(run, ea, "1/", files, &down)).await?;
                if plan.both {
                    measure(run, &ba, test, files, 1, put(run, ea, "2/", files)).await?;
                }
            }
            // Remote ↔ remote: seed A, copy A → B, then back.
            (Some(ea), Some(eb)) => {
                run.step(format!("Putting test data on {}…", ea.name), files.iter().map(|f| f.2).sum());
                put(run, ea, "1/", files).await?;
                measure(run, &ab, test, files, 2, copy(run, ea, "1/", eb, "1/", files)).await?;
                if plan.both {
                    measure(run, &ba, test, files, 2, copy(run, eb, "1/", ea, "2/", files)).await?;
                }
            }
            (None, None) => return Err("Pick at least one remote".into()),
        }
    }
    Ok(())
}

impl End {
    fn bucket_label(&self) -> String {
        if self.bucket.is_empty() { String::new() } else { format!("{}/", self.bucket) }
    }
}

/// The folder test folders are made in (`dir` without its last component).
fn parent_of(dir: &str) -> String {
    dir.trim_end_matches('/').rsplit_once('/').map(|(p, _)| format!("{p}/")).unwrap_or_default()
}

/// Deletes the test folder and checks it's gone (one more try for what's left).
async fn remove(e: &End) -> Result<(), String> {
    let first = e.remote.delete(&e.bucket, vec![e.dir.clone()], None).await;
    let left = |all: Vec<(String, u64)>| {
        all.into_iter().filter(|(k, _)| !k.ends_with('/')).map(|(k, _)| k).collect::<Vec<_>>()
    };
    let rest = match e.remote.list_all(&e.bucket, &e.dir).await {
        Ok(all) => left(all),
        Err(_) => return first.map(|_| ()), // gone (listing a missing folder fails on some servers)
    };
    if rest.is_empty() {
        return Ok(());
    }
    e.remote.delete(&e.bucket, rest, None).await?;
    let rest = e.remote.list_all(&e.bucket, &e.dir).await.map(left).unwrap_or_default();
    if rest.is_empty() {
        let _ = e.remote.delete(&e.bucket, vec![e.dir.clone()], None).await; // the folder itself
        Ok(())
    } else {
        Err(first.err().unwrap_or_else(|| format!("{} files remain", rest.len())))
    }
}

/// Removes test folders earlier runs left in this folder (the app quit before cleaning up).
async fn sweep(run: &Run, e: &End) {
    let Ok(list) = e.remote.list(&e.bucket, &parent_of(&e.dir)).await else { return };
    let mut gone = 0;
    for old in list.iter().filter(|x| x.dir && x.name.starts_with(".noexplorer-speedtest-") && x.key != e.dir) {
        let old =
            End { name: e.name.clone(), remote: e.remote.clone(), bucket: e.bucket.clone(), dir: old.key.clone() };
        if remove(&old).await.is_ok() {
            gone += 1;
        }
    }
    if gone > 0 {
        let s = if gone == 1 { "" } else { "s" };
        run.notes.lock().unwrap().push(format!("Removed {gone} test folder{s} an earlier run left in {}", e.name));
    }
}

/// Starts a speed test between `a` and `b` (at least one remote). Returns at once; progress and
/// results come as `"speedtest"` events with this `id`, and `cancel(id)` stops it.
#[tauri::command]
pub async fn speed_test(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    a: Node,
    b: Node,
    plan: Plan,
) -> Result<(), String> {
    let short = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let end = |n: &Node| -> Result<Option<End>, String> {
        let Some(acc) = n.account.as_deref() else { return Ok(None) };
        let prefix =
            if n.prefix.is_empty() || n.prefix.ends_with('/') { n.prefix.clone() } else { format!("{}/", n.prefix) };
        Ok(Some(End {
            name: n.name.clone(),
            remote: state.remote(Some(acc))?,
            bucket: n.bucket.clone(),
            dir: format!("{prefix}.noexplorer-speedtest-{short}/"),
        }))
    };
    let (ea, eb) = (end(&a)?, end(&b)?);
    if ea.is_none() && eb.is_none() {
        return Err("Pick at least one remote".into());
    }
    if let (Some(x), Some(y)) = (&ea, &eb)
        && Arc::ptr_eq(&x.remote, &y.remote)
        && x.bucket == y.bucket
        && x.dir == y.dir
    {
        return Err("Pick two different places".into());
    }
    let first = ea.as_ref().or(eb.as_ref()).unwrap();
    let job = Job::probe(&app, &id, first.remote.account(), &first.bucket);
    let tmp = crate::transfer::temp_root().join(format!("speedtest-{short}"));
    let window = app.clone();
    let run = Arc::new(Run {
        emit: Box::new(move |e| {
            let _ = window.emit("speedtest", e);
        }),
        id: id.clone(),
        job,
        phase: Mutex::new(String::new()),
        total: Default::default(),
        results: Mutex::new(vec![]),
        notes: Mutex::new(vec![]),
        tmp,
    });
    tauri::async_runtime::spawn(async move {
        let res = execute(&run, &a, &b, ea.as_ref(), eb.as_ref(), &plan).await;
        app.state::<AppState>().cancels.lock().unwrap().remove(&run.id);
        run.event(true, res.err());
    });
    Ok(())
}

/// The test, then the cleanup on every remote and in the temp folder, whatever happened.
async fn execute(
    run: &Arc<Run>,
    a: &Node,
    b: &Node,
    ea: Option<&End>,
    eb: Option<&End>,
    plan: &Plan,
) -> Result<(), String> {
    // Progress ticks while a step runs.
    let ticker = {
        let run = run.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                run.event(false, None);
            }
        })
    };
    for e in [ea, eb].into_iter().flatten() {
        *run.phase.lock().unwrap() = format!("Checking {} for old test data…", e.name);
        run.event(false, None);
        sweep(run, e).await;
    }
    let res = run_test(run, a, b, ea, eb, plan).await;
    ticker.abort();
    *run.phase.lock().unwrap() = "Cleaning up…".into();
    run.event(false, None);
    for e in [ea, eb].into_iter().flatten() {
        if let Err(left) = remove(e).await {
            run.notes.lock().unwrap().push(format!(
                "Couldn't remove all test data from {} ({}{}): {left}",
                e.name,
                e.bucket_label(),
                e.dir
            ));
        }
    }
    let _ = tokio::fs::remove_dir_all(&run.tmp).await;
    *run.phase.lock().unwrap() = String::new();
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(tmp: PathBuf) -> Arc<Run> {
        Arc::new(Run {
            emit: Box::new(|_| {}),
            id: "t".into(),
            job: Arc::new(Job::untracked("probe", "t".into(), "", "", "")),
            phase: Mutex::new(String::new()),
            total: Default::default(),
            results: Mutex::new(vec![]),
            notes: Mutex::new(vec![]),
            tmp,
        })
    }

    /// Against dev S3 and dev Nextcloud (env vars as `s3_roundtrip` / `nextcloud_roundtrip`):
    /// this computer ↔ S3 and S3 ↔ Nextcloud both ways, then nothing is left behind.
    #[tokio::test]
    async fn speed_test_against_dev_servers() {
        let (Ok(endpoint), Ok(nc)) =
            (std::env::var("NOEXPLORER_TEST_ENDPOINT"), std::env::var("NOEXPLORER_TEST_NEXTCLOUD"))
        else {
            eprintln!("skipping speed_test_against_dev_servers: NOEXPLORER_TEST_ENDPOINT / _NEXTCLOUD not set");
            return;
        };
        let s3p = crate::config::Profile {
            id: "s3".into(),
            endpoint,
            access_key: std::env::var("NOEXPLORER_TEST_KEY").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_SECRET").unwrap(),
            path_style: true,
            ..Default::default()
        };
        let ncp = crate::config::Profile {
            id: "nc".into(),
            kind: crate::config::ProviderKind::Nextcloud,
            endpoint: nc,
            access_key: std::env::var("NOEXPLORER_TEST_DAV_USER").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_DAV_PASS").unwrap(),
            ..Default::default()
        };
        let s3 = Arc::new(Remote::new(&s3p));
        let bucket = format!("nx-speed-{}", std::process::id());
        s3.create_container(&bucket).await.unwrap();
        let ncr = Arc::new(Remote::new(&ncp));
        let local = Node { name: "This computer".into(), account: None, bucket: String::new(), prefix: String::new() };
        let node =
            |n: &str| Node { name: n.into(), account: Some(n.into()), bucket: String::new(), prefix: String::new() };
        let e_s3 = End {
            name: "S3".into(),
            remote: s3.clone(),
            bucket: bucket.clone(),
            dir: ".noexplorer-speedtest-t1/".into(),
        };
        let e_nc = End {
            name: "NC".into(),
            remote: ncr.clone(),
            bucket: "Files".into(),
            dir: ".noexplorer-speedtest-t1/".into(),
        };
        let plan = Plan { big_mb: 6, small_count: 5, small_kb: 8, both: true };
        let tmp = std::env::temp_dir().join(format!("nx-speedrun-{}", uuid::Uuid::new_v4()));

        // A test folder an earlier run left behind (the app quit mid-run) is removed first.
        crate::s3::make_client(&s3p)
            .put_object()
            .bucket(&bucket)
            .key(".noexplorer-speedtest-old/large.bin")
            .body(aws_sdk_s3::primitives::ByteStream::from_static(b"x"))
            .send()
            .await
            .unwrap();
        let r = run(tmp.clone());
        execute(&r, &local, &node("S3"), None, Some(&e_s3), &plan).await.unwrap();
        let notes = r.notes.lock().unwrap().clone();
        assert_eq!(notes, ["Removed 1 test folder an earlier run left in S3"]);
        let res = r.results.lock().unwrap().clone();
        let routes: Vec<(String, &str)> = res.iter().map(|m| (m.route.clone(), m.test)).collect();
        assert_eq!(
            routes,
            [
                ("S3".to_string(), "latency"),
                ("This computer → S3".into(), "large"),
                ("S3 → This computer".into(), "large"),
                ("This computer → S3".into(), "small"),
                ("S3 → This computer".into(), "small")
            ]
        );
        assert!(res.iter().all(|m| m.secs > 0.0) && res[1].bytes == 6 << 20 && res[3].files == 5);

        let r = run(tmp.clone());
        execute(&r, &node("S3"), &node("NC"), Some(&e_s3), Some(&e_nc), &plan).await.unwrap();
        let res = r.results.lock().unwrap().clone();
        assert_eq!(res.iter().filter(|m| m.test != "latency").count(), 4, "S3 → NC and back, large and small");

        // Cleaned up: no test folder on either side, no temp files.
        assert!(s3.list_all(&bucket, "").await.unwrap().is_empty(), "S3 test folder removed");
        assert!(!ncr.list("Files", "").await.unwrap().iter().any(|e| e.name.starts_with(".noexplorer-speedtest")));
        assert!(!tmp.exists(), "temp data removed");
        crate::s3::make_client(&s3p).delete_bucket().bucket(&bucket).send().await.unwrap();
    }

    #[test]
    fn test_data_is_random_and_sized() {
        let dir = std::env::temp_dir().join(format!("nx-speed-{}", uuid::Uuid::new_v4()));
        let plan = Plan { big_mb: 2, small_count: 3, small_kb: 4, both: false };
        let (large, small) = generate(&dir, &plan).unwrap();
        assert_eq!(large[0].2, 2 << 20);
        assert_eq!(std::fs::metadata(&large[0].0).unwrap().len(), 2 << 20);
        assert_eq!(
            small.iter().map(|f| f.1.as_str()).collect::<Vec<_>>(),
            ["small/0000.bin", "small/0001.bin", "small/0002.bin"]
        );
        let a = std::fs::read(&small[0].0).unwrap();
        let b = std::fs::read(&small[1].0).unwrap();
        assert_eq!(a.len(), 4096);
        assert_ne!(a, b, "every file differs");
        assert!(a.iter().filter(|x| **x == 0).count() < 100, "not zeros");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
