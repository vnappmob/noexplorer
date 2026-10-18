//! Background transfer jobs and their progress events.
//!
//! A command creates a [`Job`], returns its id to the UI immediately, and runs the work via
//! [`Job::spawn`]. While running, a ticker emits a `"job"` event every 250 ms with a
//! [`JobEvent`] snapshot; a final event carries status `done` / `error` / `cancelled`.
//! The UI renders these in the Transfers panel (`src/jobs.ts`).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;

const TICK: Duration = Duration::from_millis(250);

pub struct Job {
    pub id: String,
    pub kind: &'static str,
    pub label: String,
    /// Profile id of the remote account.
    pub account: String,
    pub bucket: String,
    pub prefix: String,
    pub files_total: AtomicU64,
    pub files_done: AtomicU64,
    pub bytes_total: AtomicU64,
    /// Shared so streaming uploads (which must be `'static`) can count progress.
    pub bytes_done: Arc<AtomicU64>,
    errors: Mutex<Vec<String>>,
    cancel: Arc<AtomicBool>,
    /// Path to reveal when the user clicks "Show" (downloads only).
    pub dest: Mutex<Option<String>>,
    /// Local folder a download writes into; the UI refreshes the left pane when it matches.
    pub local_dir: Mutex<Option<String>>,
    /// Copy jobs (`kind = "copy"`): where the files come from (`account`/`bucket` are the target).
    pub src: Mutex<Option<(String, String)>>,
    /// Folder the files come from: the local folder (uploads) or the source prefix (copies). The
    /// transfer lanes show it when the panes have moved elsewhere. Downloads come from `prefix`.
    pub src_dir: Mutex<Option<String>>,
    /// Set from the UI (`resume::pause`); data flow waits while it's on.
    paused: Arc<AtomicBool>,
    /// Experimental `resume` was on when the job started: retries, `.part` continuation, journals.
    pub resume: bool,
}

/// What moving data needs from its job, without borrowing it (streams must be `'static`).
#[derive(Clone)]
pub struct Flow {
    done: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl Flow {
    /// `n` more bytes moved: count them, keep to the speed limit, and wait while paused.
    pub async fn flow(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
        crate::resume::throttle(n).await;
        while self.paused.load(Ordering::Relaxed) && !self.cancel.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Before sending `n` bytes in one request (a small file, a multipart part): wait while paused,
    /// then for the speed limit's share. Progress is counted afterwards with `count`.
    pub async fn gate(&self, n: u64) {
        while self.paused.load(Ordering::Relaxed) && !self.cancel.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        crate::resume::throttle(n).await;
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Payload of the `"job"` event. Mirrors `JobEvent` in `src/types.ts`.
#[derive(Serialize, Clone)]
pub struct JobEvent {
    id: String,
    kind: &'static str,
    label: String,
    account: String,
    bucket: String,
    prefix: String,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    status: &'static str,
    error: Option<String>,
    dest: Option<String>,
    local_dir: Option<String>,
    /// Copy jobs: source account and bucket.
    src_account: Option<String>,
    src_bucket: Option<String>,
    /// Uploads and copies: the folder the files come from.
    src_dir: Option<String>,
    /// Paused from the UI (experimental `resume`).
    paused: bool,
}

impl Job {
    /// A job that isn't registered for cancellation or reported to the UI.
    pub fn untracked(kind: &'static str, label: String, account: &str, bucket: &str, prefix: &str) -> Job {
        Job {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            label,
            account: account.into(),
            bucket: bucket.into(),
            prefix: prefix.into(),
            files_total: AtomicU64::new(0),
            files_done: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            bytes_done: Arc::new(AtomicU64::new(0)),
            errors: Mutex::new(vec![]),
            cancel: Arc::new(AtomicBool::new(false)),
            dest: Mutex::new(None),
            local_dir: Mutex::new(None),
            src: Mutex::new(None),
            src_dir: Mutex::new(None),
            paused: Arc::new(AtomicBool::new(false)),
            resume: false,
        }
    }

    /// A job registered in [`AppState::cancels`] so the UI can cancel it by id.
    pub fn new(
        app: &AppHandle,
        kind: &'static str,
        label: String,
        account: &str,
        bucket: &str,
        prefix: &str,
    ) -> Arc<Job> {
        let mut job = Job::untracked(kind, label, account, bucket, prefix);
        let st = app.state::<AppState>();
        job.resume = st.settings().general.experimental.resume;
        st.cancels.lock().unwrap().insert(job.id.clone(), job.cancel.clone());
        st.pauses.lock().unwrap().insert(job.id.clone(), job.paused.clone());
        Arc::new(job)
    }

    /// Progress, cancel and pause handles, for work that outlives a borrow of the job.
    pub fn flow_handle(&self) -> Flow {
        Flow { done: self.bytes_done.clone(), cancel: self.cancel.clone(), paused: self.paused.clone() }
    }

    /// `n` more bytes moved (progress, speed limit, pause). See [`Flow::flow`].
    pub async fn flow(&self, n: u64) {
        self.flow_handle().flow(n).await
    }

    #[cfg(test)]
    pub fn set_paused(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
    }

    /// Pause and speed limit before a whole request. See [`Flow::gate`].
    pub async fn gate(&self, n: u64) {
        self.flow_handle().gate(n).await
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Records a per-file failure; the job keeps going with the other files.
    pub fn fail(&self, what: &str, e: String) {
        self.errors.lock().unwrap().push(format!("{what}: {e}"));
    }

    /// Pushes a snapshot to the UI (`"job"` event) and to the tray menu.
    fn publish(&self, app: &AppHandle, status: &'static str) {
        let ev = self.event(status);
        crate::tray::on_job(app, &ev.id, ev.kind, &ev.label, ev.bytes_done, ev.bytes_total, status);
        let _ = app.emit("job", ev);
    }

    fn event(&self, status: &'static str) -> JobEvent {
        let errors = self.errors.lock().unwrap();
        // One lock, taken before the literal: a guard used inside a struct literal lives until the
        // end of the statement, so locking `src` twice there deadlocks (v0.4.4 hung every job).
        let (src_account, src_bucket) = self.src.lock().unwrap().clone().unzip();
        JobEvent {
            id: self.id.clone(),
            kind: self.kind,
            label: self.label.clone(),
            account: self.account.clone(),
            bucket: self.bucket.clone(),
            prefix: self.prefix.clone(),
            files_done: self.files_done.load(Ordering::Relaxed),
            files_total: self.files_total.load(Ordering::Relaxed),
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            bytes_total: self.bytes_total.load(Ordering::Relaxed),
            status,
            error: match errors.len() {
                0 => None,
                1 => Some(errors[0].clone()),
                n => Some(format!("{} errors. First: {}", n, errors[0])),
            },
            dest: self.dest.lock().unwrap().clone(),
            local_dir: self.local_dir.lock().unwrap().clone(),
            src_account,
            src_bucket,
            src_dir: self.src_dir.lock().unwrap().clone(),
            paused: self.paused.load(Ordering::Relaxed),
        }
    }

    /// Runs `work` in the background while emitting throttled progress events, then a final one.
    pub fn spawn<F>(self: Arc<Self>, app: AppHandle, work: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let job = self;
        tauri::async_runtime::spawn(async move {
            let finished = Arc::new(AtomicBool::new(false));
            let ticker = {
                let (job, app, finished) = (job.clone(), app.clone(), finished.clone());
                tauri::async_runtime::spawn(async move {
                    while !finished.load(Ordering::Relaxed) {
                        job.publish(&app, "running");
                        tokio::time::sleep(TICK).await;
                    }
                })
            };
            work.await;
            finished.store(true, Ordering::Relaxed);
            let _ = ticker.await;
            let status = if job.cancelled() {
                "cancelled"
            } else if job.errors.lock().unwrap().is_empty() {
                "done"
            } else {
                "error"
            };
            job.publish(&app, status);
            let st = app.state::<AppState>();
            st.cancels.lock().unwrap().remove(&job.id);
            st.pauses.lock().unwrap().remove(&job.id);
            // A finished or cancelled transfer has nothing left to resume (errors stay resumable).
            if job.resume && status != "error" {
                crate::resume::pending_remove(&job.id);
            }
        });
    }
}

#[tauri::command]
pub fn cancel(state: State<'_, AppState>, id: String) {
    if let Some(f) = state.cancels.lock().unwrap().get(&id) {
        f.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (v0.4.4): `event()` deadlocked on a copy job's `src`, so no job ever reported.
    #[test]
    fn event_snapshot_does_not_deadlock() {
        let job = Job::untracked("copy", "a.txt".into(), "acc", "dst", "");
        *job.src.lock().unwrap() = Some(("src-acc".into(), "src-bucket".into()));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ev = job.event("running");
            tx.send((ev.src_account, ev.src_bucket)).unwrap();
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("Job::event deadlocked");
        assert_eq!(got, (Some("src-acc".into()), Some("src-bucket".into())));
    }
}
