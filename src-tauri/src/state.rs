//! Shared application state, registered with `app.manage(...)`.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::config::{Profile, Settings};
use crate::provider::Remote;

#[derive(Default)]
pub struct AppState {
    settings: Mutex<Settings>,
    /// One client per account (built on first use), with the profile it was built from.
    remotes: Mutex<HashMap<String, (Profile, Arc<Remote>)>>,
    /// Cancel flags of running jobs, keyed by job id.
    pub cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Pause flags of running jobs, keyed by job id.
    pub pauses: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AppState {
    /// Installs new settings. Clients of accounts that changed or were removed are dropped
    /// (rebuilt on next use); unchanged accounts keep theirs, with their caches (AWS regions).
    /// Running jobs hold their own `Arc<Remote>`, so they finish on the client they started with.
    pub fn set_settings(&self, s: Settings) {
        self.remotes.lock().unwrap().retain(|id, (p, _)| s.profiles.iter().any(|q| q.id == *id && q == p));
        *self.settings.lock().unwrap() = s;
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Drops the cached client of `account` (after a new sign-in, so the next use reads the new tokens).
    pub fn forget(&self, account: &str) {
        self.remotes.lock().unwrap().remove(account);
    }

    /// The client for `account` (a profile id), or for the active account when `None`/empty.
    /// Always use this to reach remote storage (ADR-025): it applies per-account caches such as
    /// AWS per-bucket regions (ADR-017).
    pub fn remote(&self, account: Option<&str>) -> Result<Arc<Remote>, String> {
        let s = self.settings.lock().unwrap();
        let p = match account.filter(|a| !a.is_empty()) {
            Some(id) => s.profiles.iter().find(|p| p.id == id).ok_or("That account no longer exists")?,
            None => s.active_profile().ok_or("No account configured — open Settings first")?,
        };
        let mut remotes = self.remotes.lock().unwrap();
        let (_, r) = remotes.entry(p.id.clone()).or_insert_with(|| (p.clone(), Arc::new(Remote::new(p))));
        Ok(r.clone())
    }
}
