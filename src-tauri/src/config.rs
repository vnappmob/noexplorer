//! Connection settings: S3 accounts ("profiles") persisted as JSON in the OS app-config dir,
//! plus S3 client construction.
//!
//! File format (v2): `{ "profiles": [Profile…], "active": "<profile id>", "download_dir": "" }`.
//! v1 files (a single flat connection object) are migrated on load, see [`parse_settings`].

use std::path::PathBuf;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{
    BehaviorVersion, Builder, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;
use crate::util::err;

/// One S3 account. Mirrors `Profile` in `src/types.ts`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Profile {
    /// Stable id (uuid), assigned on first save.
    pub id: String,
    /// Display name in the account picker.
    pub name: String,
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// Bucket to open first (optional).
    pub bucket: String,
    /// Path-style addressing (`https://host/bucket/key`). Required by MinIO & most self-hosted S3.
    pub path_style: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            endpoint: String::new(),
            region: "us-east-1".into(),
            access_key: String::new(),
            secret_key: String::new(),
            bucket: String::new(),
            path_style: true,
        }
    }
}

/// App behavior (Settings → General). Mirrors `General` in `src/types.ts`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct General {
    /// Closing the window hides it to the tray/menu bar; transfers keep running.
    pub run_in_background: bool,
    /// Check GitHub Releases for a new version at startup and every few hours.
    pub check_updates: bool,
    /// System notification when a transfer finishes while the window is hidden.
    pub notify: bool,
}

impl Default for General {
    fn default() -> Self {
        Self { run_in_background: true, check_updates: true, notify: true }
    }
}

/// Everything in `config.json`. Mirrors `Settings` in `src/types.ts`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub profiles: Vec<Profile>,
    /// Id of the profile shown in the remote pane.
    pub active: String,
    /// Fallback download folder when the UI doesn't pass one (empty = OS Downloads).
    pub download_dir: String,
    pub general: General,
}

impl Settings {
    pub fn active_profile(&self) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == self.active).or(self.profiles.first())
    }

    /// Inserts or replaces `p` (matched by id; a new id is assigned when empty). Returns its id.
    pub fn upsert(&mut self, mut p: Profile) -> String {
        if p.id.is_empty() {
            p.id = uuid::Uuid::new_v4().to_string();
        }
        if p.name.trim().is_empty() {
            p.name = default_name(&p);
        }
        let id = p.id.clone();
        match self.profiles.iter_mut().find(|x| x.id == id) {
            Some(slot) => *slot = p,
            None => self.profiles.push(p),
        }
        if self.active.is_empty() {
            self.active = id.clone();
        }
        id
    }

    /// Removes a profile; if it was active, the first remaining one becomes active.
    pub fn remove(&mut self, id: &str) {
        self.profiles.retain(|p| p.id != id);
        if self.active == id || !self.profiles.iter().any(|p| p.id == self.active) {
            self.active = self.profiles.first().map(|p| p.id.clone()).unwrap_or_default();
        }
    }
}

/// "minio.example.com" from "https://minio.example.com:9000/" — used when the user leaves Name empty.
fn default_name(p: &Profile) -> String {
    let e = normalize_endpoint(&p.endpoint);
    let host = e.split("://").nth(1).unwrap_or(&e).split(['/', ':']).next().unwrap_or("");
    if host.is_empty() { "S3".into() } else { host.into() }
}

/// Reads either format. v1 = one flat `{endpoint, access_key, …, download_dir}` object.
pub fn parse_settings(bytes: &[u8]) -> Option<Settings> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if v.get("profiles").is_some() {
        return serde_json::from_value(v).ok();
    }
    // v1 → v2 migration.
    let download_dir = v.get("download_dir").and_then(|d| d.as_str()).unwrap_or_default().to_string();
    let legacy: Profile = serde_json::from_value(v).ok()?;
    let mut s = Settings { download_dir, ..Default::default() };
    if !legacy.endpoint.is_empty() {
        s.upsert(legacy);
    }
    Some(s)
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

pub fn load_settings(app: &AppHandle) -> Settings {
    config_path(app).ok().and_then(|p| std::fs::read(p).ok()).and_then(|b| parse_settings(&b)).unwrap_or_default()
}

fn write_settings(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let p = config_path(app)?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, serde_json::to_vec_pretty(s).unwrap()).map_err(|e| e.to_string())?;
    // The file holds secret keys: keep it private to the user.
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

pub fn make_client(p: &Profile) -> Client {
    let region = match p.region.trim() {
        "" => "us-east-1".to_string(),
        r => r.to_string(),
    };
    let mut b = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region))
        .credentials_provider(Credentials::new(p.access_key.trim(), p.secret_key.trim(), None, None, "noexplorer"))
        .force_path_style(p.path_style)
        // Only send/verify checksums when an API requires them: newer SDK defaults (CRC32 on
        // every request) break some self-hosted / older S3-compatible servers. See ADR-004.
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired);
    let endpoint = normalize_endpoint(&p.endpoint);
    if !endpoint.is_empty() {
        b = b.endpoint_url(endpoint);
    }
    Client::from_conf(b.build())
}

// ---------- commands ----------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Checks a connection and returns the buckets visible with these credentials.
#[tauri::command]
pub async fn test_profile(profile: Profile) -> Result<Vec<String>, String> {
    let c = make_client(&profile);
    match c.list_buckets().send().await {
        Ok(r) => Ok(r.buckets().iter().filter_map(|b| b.name().map(String::from)).collect()),
        Err(e) if !profile.bucket.trim().is_empty() => {
            // The key may lack s3:ListAllMyBuckets; fall back to the configured bucket.
            c.head_bucket().bucket(profile.bucket.trim()).send().await.map_err(|_| err(e))?;
            Ok(vec![profile.bucket.trim().to_string()])
        }
        Err(e) => Err(err(e)),
    }
}

fn commit(app: &AppHandle, state: &AppState, s: Settings) -> Result<Settings, String> {
    write_settings(app, &s)?;
    state.set_settings(s.clone());
    Ok(s)
}

/// Adds or updates an account and makes it the active one.
#[tauri::command]
pub fn save_profile(app: AppHandle, state: State<'_, AppState>, profile: Profile) -> Result<Settings, String> {
    let mut s = state.settings();
    s.active = s.upsert(profile);
    commit(&app, &state, s)
}

#[tauri::command]
pub fn delete_profile(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<Settings, String> {
    let mut s = state.settings();
    s.remove(&id);
    commit(&app, &state, s)
}

#[tauri::command]
pub fn set_active_profile(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<Settings, String> {
    let mut s = state.settings();
    if !s.profiles.iter().any(|p| p.id == id) {
        return Err("Unknown account".into());
    }
    s.active = id;
    commit(&app, &state, s)
}

#[tauri::command]
pub fn save_general(app: AppHandle, state: State<'_, AppState>, general: General) -> Result<Settings, String> {
    let mut s = state.settings();
    s.general = general;
    commit(&app, &state, s)
}

/// Buckets of the active account.
#[tauri::command]
pub async fn list_buckets(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let p = state.settings().active_profile().cloned().ok_or("No account configured")?;
    test_profile(p).await
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
    fn migrates_v1_single_connection() {
        let v1 = br#"{"endpoint":"http://nas:9000","access_key":"k","secret_key":"s","bucket":"b","path_style":true,"download_dir":"/dl"}"#;
        let s = parse_settings(v1).unwrap();
        assert_eq!(s.profiles.len(), 1);
        let p = &s.profiles[0];
        assert_eq!((p.endpoint.as_str(), p.bucket.as_str(), p.name.as_str()), ("http://nas:9000", "b", "nas"));
        assert!(!p.id.is_empty() && s.active == p.id);
        assert_eq!(s.download_dir, "/dl");
    }

    #[test]
    fn v2_roundtrip_and_partial_profiles_default_to_path_style() {
        let s = parse_settings(br#"{"profiles":[{"id":"a","endpoint":"http://x"}],"active":"a"}"#).unwrap();
        assert!(s.profiles[0].path_style, "path-style must default to on");
        assert!(s.general.run_in_background && s.general.check_updates, "general defaults to on");
        assert_eq!(s.profiles[0].region, "us-east-1");
        let again = parse_settings(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(again, s);
    }

    #[test]
    fn upsert_and_remove_keep_a_valid_active_profile() {
        let mut s = Settings::default();
        let a = s.upsert(Profile { endpoint: "https://a.example.com".into(), ..Default::default() });
        let b = s.upsert(Profile { name: "B".into(), ..Default::default() });
        assert_eq!(s.active, a, "first profile becomes active");
        assert_eq!(s.profiles[0].name, "a.example.com");
        s.upsert(Profile { id: b.clone(), name: "B2".into(), ..Default::default() });
        assert_eq!(s.profiles.len(), 2);
        assert_eq!(s.profiles[1].name, "B2");
        s.remove(&a);
        assert_eq!(s.active, b);
        s.remove(&b);
        assert!(s.active.is_empty() && s.active_profile().is_none());
    }
}
