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
    if is_aws(p) {
        return "AWS S3".into();
    }
    let host = endpoint_host(&p.endpoint);
    if host.is_empty() { "S3".into() } else { host }
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

/// Host part of a normalized endpoint ("https://s3.eu-west-1.amazonaws.com:443/" → "s3.eu-west-1.amazonaws.com").
pub fn endpoint_host(endpoint: &str) -> String {
    let e = normalize_endpoint(endpoint);
    let rest = e.split("://").nth(1).unwrap_or(&e);
    rest.split(['/', ':']).next().unwrap_or("").to_ascii_lowercase()
}

/// AWS S3 itself: no endpoint, or an `*.amazonaws.com(.cn)` endpoint. For AWS the endpoint is
/// resolved from the region by the SDK, and each bucket is used in its own region (ADR-017).
pub fn is_aws(p: &Profile) -> bool {
    let h = endpoint_host(&p.endpoint);
    h.is_empty() || h.ends_with(".amazonaws.com") || h.ends_with(".amazonaws.com.cn")
}

/// Region encoded in a provider's regional endpoint, if any:
/// `s3.ap-southeast-1.amazonaws.com`, `s3-ap-southeast-1.amazonaws.com`, `s3.dualstack.X.amazonaws.com`,
/// `s3.X.wasabisys.com`, `s3.X.backblazeb2.com`, `X.digitaloceanspaces.com`, `X.linodeobjects.com`.
pub fn region_from_endpoint(endpoint: &str) -> Option<String> {
    let h = endpoint_host(endpoint);
    let labels: Vec<&str> = h.split('.').collect();
    let looks_like_region = |r: &str| r.contains('-') && r.chars().any(|c| c.is_ascii_digit());
    let pick = |r: &str| looks_like_region(r).then(|| r.to_string());
    if h.ends_with(".amazonaws.com") || h.ends_with(".amazonaws.com.cn") {
        return match labels.as_slice() {
            ["s3", "dualstack", r, ..] => pick(r),
            ["s3", r, "amazonaws", ..] => pick(r),
            [first, "amazonaws", ..] => first.strip_prefix("s3-").and_then(pick),
            _ => None,
        };
    }
    match labels.as_slice() {
        ["s3", r, "wasabisys", "com"] | ["s3", r, "backblazeb2", "com"] => pick(r),
        // DO/Linode regions have no dash ("sgp1", "us-east-1" for Linode is fine too).
        [r, "digitaloceanspaces", "com"] | [r, "linodeobjects", "com"] if r.chars().any(|c| c.is_ascii_digit()) => {
            Some(r.to_string())
        }
        _ => None,
    }
}

/// Region used to sign requests: the endpoint's region wins (a mismatch can never work),
/// then the Region field, then us-east-1.
pub fn effective_region(p: &Profile) -> String {
    region_from_endpoint(&p.endpoint).unwrap_or_else(|| match p.region.trim() {
        "" => "us-east-1".to_string(),
        r => r.to_string(),
    })
}

pub fn make_client(p: &Profile) -> Client {
    make_client_in(p, &effective_region(p))
}

/// Client signing for `region`. AWS profiles ignore the endpoint field (the SDK picks
/// `s3.<region>.amazonaws.com`); other providers always use their endpoint.
pub fn make_client_in(p: &Profile, region: &str) -> Client {
    let mut b = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region.to_string()))
        .credentials_provider(Credentials::new(p.access_key.trim(), p.secret_key.trim(), None, None, "noexplorer"))
        .force_path_style(p.path_style)
        // Only send/verify checksums when an API requires them: newer SDK defaults (CRC32 on
        // every request) break some self-hosted / older S3-compatible servers. See ADR-004.
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired);
    let endpoint = normalize_endpoint(&p.endpoint);
    if !endpoint.is_empty() && !is_aws(p) {
        b = b.endpoint_url(endpoint);
    }
    Client::from_conf(b.build())
}

/// AWS: the region a bucket lives in. HEAD Bucket answers with `x-amz-bucket-region`, even
/// on the 301 you get when asking the wrong region, so one request is enough.
pub async fn detect_bucket_region(p: &Profile, bucket: &str) -> Option<String> {
    let c = make_client_in(p, "us-east-1");
    match c.head_bucket().bucket(bucket).send().await {
        Ok(o) => o.bucket_region().map(String::from),
        Err(e) => e.raw_response().and_then(|r| r.headers().get("x-amz-bucket-region")).map(String::from),
    }
}

/// Bucket names (+ region when the server reports it, as AWS does) visible to these credentials.
pub async fn list_buckets_with_regions(p: &Profile) -> Result<Vec<(String, Option<String>)>, String> {
    let c = make_client(p);
    match c.list_buckets().send().await {
        Ok(r) => Ok(r
            .buckets()
            .iter()
            .filter_map(|b| b.name().map(|n| (n.to_string(), b.bucket_region().map(String::from))))
            .collect()),
        Err(e) if !p.bucket.trim().is_empty() => {
            // The key may lack s3:ListAllMyBuckets; fall back to the configured bucket.
            let bucket = p.bucket.trim();
            let region = if is_aws(p) { detect_bucket_region(p, bucket).await } else { None };
            let check = region.as_deref().map(|r| make_client_in(p, r)).unwrap_or(c);
            check.head_bucket().bucket(bucket).send().await.map_err(|_| err(e))?;
            Ok(vec![(bucket.to_string(), region)])
        }
        Err(e) => Err(err(e)),
    }
}

// ---------- commands ----------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Checks a connection and returns the buckets visible with these credentials.
#[tauri::command]
pub async fn test_profile(profile: Profile) -> Result<Vec<String>, String> {
    Ok(list_buckets_with_regions(&profile).await?.into_iter().map(|(n, _)| n).collect())
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
    let buckets = list_buckets_with_regions(&p).await?;
    for (name, region) in &buckets {
        if let Some(r) = region {
            state.remember_region(name, r); // saves a HEAD per bucket on AWS
        }
    }
    Ok(buckets.into_iter().map(|(n, _)| n).collect())
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
    fn aws_detection_and_region_inference() {
        let p = |endpoint: &str, region: &str| Profile {
            endpoint: endpoint.into(),
            region: region.into(),
            ..Default::default()
        };
        assert!(is_aws(&p("", "")));
        assert!(is_aws(&p("s3.amazonaws.com", "")));
        assert!(is_aws(&p("https://s3.ap-southeast-1.amazonaws.com/", "")));
        assert!(!is_aws(&p("http://nas:9000", "")));
        assert!(!is_aws(&p("https://abc.r2.cloudflarestorage.com", "auto")));
        for (e, r) in [
            ("s3.ap-southeast-1.amazonaws.com", "ap-southeast-1"),
            ("https://s3-eu-west-1.amazonaws.com", "eu-west-1"),
            ("s3.dualstack.us-west-2.amazonaws.com", "us-west-2"),
            ("s3.cn-north-1.amazonaws.com.cn", "cn-north-1"),
            ("https://s3.eu-central-2.wasabisys.com", "eu-central-2"),
            ("s3.us-west-004.backblazeb2.com", "us-west-004"),
            ("https://sgp1.digitaloceanspaces.com", "sgp1"),
        ] {
            assert_eq!(region_from_endpoint(e).as_deref(), Some(r), "{e}");
        }
        assert_eq!(region_from_endpoint("s3.amazonaws.com"), None);
        assert_eq!(region_from_endpoint("http://nas:9000"), None);
        // The endpoint's region overrides a mismatching Region field (the bug users hit).
        assert_eq!(effective_region(&p("s3.ap-southeast-1.amazonaws.com", "us-east-1")), "ap-southeast-1");
        assert_eq!(effective_region(&p("s3.amazonaws.com", "ap-southeast-1")), "ap-southeast-1");
        assert_eq!(effective_region(&p("http://nas:9000", "")), "us-east-1");
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
