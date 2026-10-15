//! Shared application state, registered with `app.manage(...)`.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use aws_sdk_s3::Client;

use crate::config::{Config, make_client};

#[derive(Default)]
pub struct AppState {
    cfg: Mutex<Option<Config>>,
    client: Mutex<Option<Client>>,
    /// Cancel flags of running jobs, keyed by job id.
    pub cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AppState {
    /// Installs a new config and rebuilds the S3 client from it.
    pub fn set_config(&self, cfg: Config) {
        *self.client.lock().unwrap() = Some(make_client(&cfg));
        *self.cfg.lock().unwrap() = Some(cfg);
    }

    /// The saved config, if the user has connected at least once.
    pub fn config(&self) -> Option<Config> {
        self.cfg.lock().unwrap().clone()
    }

    pub fn client(&self) -> Result<Client, String> {
        self.client.lock().unwrap().clone().ok_or_else(|| "Not connected — open Settings first".to_string())
    }
}
