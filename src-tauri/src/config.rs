//! Connection settings: persisted as JSON in the OS app-config dir, plus S3 client setup.

use std::path::PathBuf;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{
    BehaviorVersion, Builder, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;
use crate::util::err;

/// Mirrors the `Config` type in `src/types.ts`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// Bucket to open first (optional).
    pub bucket: String,
    /// Path-style addressing (`https://host/bucket/key`). Required by MinIO & most self-hosted S3.
    pub path_style: bool,
    /// Fallback download folder when the UI doesn't pass one (empty = OS Downloads).
    pub download_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            region: "us-east-1".into(),
            access_key: String::new(),
            secret_key: String::new(),
            bucket: String::new(),
            path_style: true,
            download_dir: String::new(),
        }
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

pub fn load_config(app: &AppHandle) -> Option<Config> {
    let p = config_path(app).ok()?;
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn write_config(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let p = config_path(app)?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, serde_json::to_vec_pretty(cfg).unwrap()).map_err(|e| e.to_string())?;
    // The file holds the secret key: keep it private to the user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Normalizes a user-typed endpoint: trims, drops a trailing `/`, defaults to https.
pub fn normalize_endpoint(raw: &str) -> String {
    let e = raw.trim().trim_end_matches('/');
    if e.is_empty() || e.contains("://") { e.to_string() } else { format!("https://{e}") }
}

pub fn make_client(cfg: &Config) -> Client {
    let region = match cfg.region.trim() {
        "" => "us-east-1".to_string(),
        r => r.to_string(),
    };
    let mut b = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region))
        .credentials_provider(Credentials::new(cfg.access_key.trim(), cfg.secret_key.trim(), None, None, "noexplorer"))
        .force_path_style(cfg.path_style)
        // Only send/verify checksums when an API requires them: newer SDK defaults (CRC32 on
        // every request) break some self-hosted / older S3-compatible servers. See ADR-004.
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired);
    let endpoint = normalize_endpoint(&cfg.endpoint);
    if !endpoint.is_empty() {
        b = b.endpoint_url(endpoint);
    }
    Client::from_conf(b.build())
}

// ---------- commands ----------

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Option<Config> {
    state.config()
}

/// Checks the connection and returns the buckets visible with these credentials.
#[tauri::command]
pub async fn test_config(cfg: Config) -> Result<Vec<String>, String> {
    let c = make_client(&cfg);
    match c.list_buckets().send().await {
        Ok(r) => Ok(r.buckets().iter().filter_map(|b| b.name().map(String::from)).collect()),
        Err(e) if !cfg.bucket.trim().is_empty() => {
            // The key may lack s3:ListAllMyBuckets; fall back to the configured bucket.
            c.head_bucket().bucket(cfg.bucket.trim()).send().await.map_err(|_| err(e))?;
            Ok(vec![cfg.bucket.trim().to_string()])
        }
        Err(e) => Err(err(e)),
    }
}

#[tauri::command]
pub fn save_config(app: AppHandle, state: State<'_, AppState>, cfg: Config) -> Result<(), String> {
    write_config(&app, &cfg)?;
    state.set_config(cfg);
    Ok(())
}

#[tauri::command]
pub async fn list_buckets(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    test_config(state.config().unwrap_or_default()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_normalization() {
        assert_eq!(normalize_endpoint(" minio.local:9000/ "), "https://minio.local:9000");
        assert_eq!(normalize_endpoint("http://10.0.0.2:9000"), "http://10.0.0.2:9000");
        assert_eq!(normalize_endpoint(""), "");
    }

    #[test]
    fn config_defaults_and_partial_json() {
        let c: Config = serde_json::from_str(r#"{"endpoint":"http://x"}"#).unwrap();
        assert!(c.path_style, "path-style must default to on");
        assert_eq!(c.region, "us-east-1");
    }
}
