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
    pub bucket: String,
    pub prefix: String,
    pub files_total: AtomicU64,
    pub files_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub bytes_done: AtomicU64,
    errors: Mutex<Vec<String>>,
    cancel: Arc<AtomicBool>,
    /// Path to reveal when the user clicks "Show" (downloads only).
    pub dest: Mutex<Option<String>>,
    /// Local folder a download writes into; the UI refreshes the left pane when it matches.
    pub local_dir: Mutex<Option<String>>,
}

/// Payload of the `"job"` event. Mirrors `JobEvent` in `src/types.ts`.
#[derive(Serialize, Clone)]
pub struct JobEvent {
    id: String,
    kind: &'static str,
    label: String,
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
}

impl Job {
    /// A job that isn't registered for cancellation or reported to the UI.
    pub fn untracked(kind: &'static str, label: String, bucket: &str, prefix: &str) -> Job {
        Job {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            label,
            bucket: bucket.into(),
            prefix: prefix.into(),
            files_total: AtomicU64::new(0),
            files_done: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            bytes_done: AtomicU64::new(0),
            errors: Mutex::new(vec![]),
            cancel: Arc::new(AtomicBool::new(false)),
            dest: Mutex::new(None),
            local_dir: Mutex::new(None),
        }
    }

    /// A job registered in [`AppState::cancels`] so the UI can cancel it by id.
    pub fn new(app: &AppHandle, kind: &'static str, label: String, bucket: &str, prefix: &str) -> Arc<Job> {
        let job = Job::untracked(kind, label, bucket, prefix);
        app.state::<AppState>().cancels.lock().unwrap().insert(job.id.clone(), job.cancel.clone());
        Arc::new(job)
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Records a per-file failure; the job keeps going with the other files.
    pub fn fail(&self, what: &str, e: String) {
        self.errors.lock().unwrap().push(format!("{what}: {e}"));
    }

    fn event(&self, status: &'static str) -> JobEvent {
        let errors = self.errors.lock().unwrap();
        JobEvent {
            id: self.id.clone(),
            kind: self.kind,
            label: self.label.clone(),
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
                        let _ = app.emit("job", job.event("running"));
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
            let _ = app.emit("job", job.event(status));
            app.state::<AppState>().cancels.lock().unwrap().remove(&job.id);
        });
    }
}

#[tauri::command]
pub fn cancel(state: State<'_, AppState>, id: String) {
    if let Some(f) = state.cancels.lock().unwrap().get(&id) {
        f.store(true, Ordering::Relaxed);
    }
}
