//! Shared application state, registered with `app.manage(...)`.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use aws_sdk_s3::Client;

use crate::config::{Settings, make_client};

#[derive(Default)]
pub struct AppState {
    settings: Mutex<Settings>,
    /// Client for the active profile; rebuilt whenever settings change.
    client: Mutex<Option<Client>>,
    /// Cancel flags of running jobs, keyed by job id.
    pub cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AppState {
    /// Installs new settings and rebuilds the S3 client for the active profile.
    pub fn set_settings(&self, s: Settings) {
        *self.client.lock().unwrap() = s.active_profile().map(make_client);
        *self.settings.lock().unwrap() = s;
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Client for the active account. Jobs clone it at start, so switching accounts
    /// mid-transfer doesn't affect running transfers.
    pub fn client(&self) -> Result<Client, String> {
        self.client.lock().unwrap().clone().ok_or_else(|| "No account configured — open Settings first".to_string())
    }
}
