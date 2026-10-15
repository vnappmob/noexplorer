//! Shared application state, registered with `app.manage(...)`.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use aws_sdk_s3::Client;

use crate::config::{Profile, Settings, detect_bucket_region, effective_region, is_aws, make_client, make_client_in};

#[derive(Default)]
pub struct AppState {
    settings: Mutex<Settings>,
    /// Client for the active profile (its effective region); rebuilt whenever settings change.
    client: Mutex<Option<Client>>,
    /// AWS only: bucket → region, learned from ListBuckets or HEAD Bucket.
    regions: Mutex<HashMap<String, String>>,
    /// AWS only: extra clients for buckets outside the profile's region, keyed by region.
    regional: Mutex<HashMap<String, Client>>,
    /// Cancel flags of running jobs, keyed by job id.
    pub cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AppState {
    /// Installs new settings and rebuilds the S3 client for the active profile.
    pub fn set_settings(&self, s: Settings) {
        *self.client.lock().unwrap() = s.active_profile().map(make_client);
        self.regions.lock().unwrap().clear();
        self.regional.lock().unwrap().clear();
        *self.settings.lock().unwrap() = s;
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn profile(&self) -> Option<Profile> {
        self.settings.lock().unwrap().active_profile().cloned()
    }

    /// Client for the active account's default region.
    pub fn client(&self) -> Result<Client, String> {
        self.client.lock().unwrap().clone().ok_or_else(|| "No account configured — open Settings first".to_string())
    }

    pub fn remember_region(&self, bucket: &str, region: &str) {
        self.regions.lock().unwrap().insert(bucket.to_string(), region.to_string());
    }

    /// Client to use for `bucket`. On AWS each bucket must be addressed in its own region
    /// (signing with another region fails), so the region is detected once and cached.
    /// Other providers have one endpoint: always the default client. Jobs clone the client at
    /// start, so switching accounts mid-transfer doesn't affect running transfers.
    pub async fn client_for(&self, bucket: &str) -> Result<Client, String> {
        let base = self.client()?;
        let Some(p) = self.profile() else { return Ok(base) };
        if !is_aws(&p) || bucket.is_empty() {
            return Ok(base);
        }
        let cached = self.regions.lock().unwrap().get(bucket).cloned();
        let region = match cached {
            Some(r) => r,
            None => {
                let r = detect_bucket_region(&p, bucket).await.unwrap_or_else(|| effective_region(&p));
                self.remember_region(bucket, &r);
                r
            }
        };
        if region == effective_region(&p) {
            return Ok(base);
        }
        let mut regional = self.regional.lock().unwrap();
        Ok(regional.entry(region.clone()).or_insert_with(|| make_client_in(&p, &region)).clone())
    }
}
