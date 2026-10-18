//! Connection settings: remote accounts ("profiles") persisted as JSON in the OS app-config dir,
//! plus endpoint/region helpers. Clients are built per account in `provider.rs` / `s3.rs`.
//!
//! File format (v2): `{ "profiles": [Profile…], "active": "<profile id>", "download_dir": "" }`.
//! v1 files (a single flat connection object) are migrated on load, see [`parse_settings`].
//! Secret keys live in the OS credential store (`secrets.rs`, ADR-020) and never go to the UI:
//! every command returns [`redacted`] settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::provider::Remote;
use crate::secrets::{OsStore, SecretStore};
use crate::state::AppState;

/// Which provider an account uses (`provider.rs`). Mirrors `ProviderKind` in `src/types.ts`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    S3,
    /// Nextcloud / ownCloud: WebDAV under `remote.php/dav`, Login Flow v2, chunked upload, shares.
    Nextcloud,
    /// Any WebDAV server; `endpoint` is the full collection URL.
    Webdav,
    /// Box with the user's own OAuth app: `access_key` = client ID, `secret_key` = client secret,
    /// refresh token in the keychain (`box_api.rs`).
    Box,
    /// Azure Blob Storage (experimental): `access_key` = storage account name, `secret_key` =
    /// account key (`azure.rs`).
    Azure,
    /// SFTP (experimental): `endpoint` = `sftp://user@host:port/path`; login with SSH keys / agent
    /// through the system `ssh` (`sftp.rs`).
    Sftp,
}

/// One remote account. Mirrors `Profile` in `src/types.ts`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Profile {
    /// Stable id (uuid), assigned on first save.
    pub id: String,
    /// Older files have no `kind`: they are S3 accounts.
    pub kind: ProviderKind,
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
    /// The secret key is in the OS credential store; config.json keeps `secret_key` empty.
    pub keychain: bool,
    /// SHA-256 fingerprint ("AB:CD:…") of the server certificate this account trusts even though
    /// the system doesn't (self-signed HTTPS, experimental; tls.rs). Empty = normal checks only.
    pub cert_pin: String,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: ProviderKind::S3,
            name: String::new(),
            endpoint: String::new(),
            region: "us-east-1".into(),
            access_key: String::new(),
            secret_key: String::new(),
            bucket: String::new(),
            path_style: true,
            keychain: false,
            cert_pin: String::new(),
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
    /// Compute folder sizes in the local pane automatically (sizes.rs).
    pub auto_sizes: bool,
    /// Settings → Experimental: features that ship off until the user turns them on.
    pub experimental: Experimental,
    /// Transfer speed cap in KB/s (0 = no limit). Used with `experimental.resume`.
    pub speed_limit_kbps: u64,
}

impl Default for General {
    fn default() -> Self {
        Self {
            run_in_background: true,
            check_updates: true,
            auto_sizes: true,
            experimental: Experimental::default(),
            speed_limit_kbps: 0,
        }
    }
}

/// Experimental features (all off by default). Mirrors `Experimental` in `src/types.ts`; the
/// Settings tab lists them from `EXPERIMENTS` in `src/experimental.ts`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Experimental {
    /// Rename, move, duplicate and undo, on both sides.
    pub file_ops: bool,
    /// Resumable transfers: continue after interruptions, retry, pause, speed limit.
    pub resume: bool,
    /// Accounts may trust their server's self-signed HTTPS certificate.
    pub self_signed: bool,
    /// Saved local ↔ remote folder pairs: Compare and upload in one click.
    pub backup_pairs: bool,
    /// Thumbnail grid view.
    pub grid: bool,
    /// Tabs, back/forward history and favorites.
    pub tabs: bool,
    /// SFTP, Google Cloud Storage and Azure Blob accounts.
    pub providers: bool,
    /// Native window material (Windows Mica, macOS vibrancy).
    pub native_look: bool,
    /// Vietnamese interface.
    pub vietnamese: bool,
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
    if p.kind == ProviderKind::S3 && is_aws(p) {
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

/// Loads config.json and fills in the secrets kept in the OS store. Secrets still in the file
/// (written before 0.4, or when the store was unavailable) are moved to the store right away.
pub fn load_settings(app: &AppHandle) -> Settings {
    let store = OsStore::new(app);
    let mut s =
        config_path(app).ok().and_then(|p| std::fs::read(p).ok()).and_then(|b| parse_settings(&b)).unwrap_or_default();
    load_secrets(&mut s, &store);
    if s.profiles.iter().any(|p| !p.keychain && !p.secret_key.is_empty()) {
        let _ = write_settings(app, &mut s);
    }
    s
}

/// Copy of `s` for the UI: secret keys never leave the backend.
pub fn redacted(s: &Settings) -> Settings {
    let mut s = s.clone();
    for p in &mut s.profiles {
        p.secret_key.clear();
    }
    s
}

fn load_secrets(s: &mut Settings, store: &dyn SecretStore) {
    for p in s.profiles.iter_mut().filter(|p| p.keychain) {
        // Unreadable (access denied, store gone): leave it empty; the user re-enters it in Settings.
        p.secret_key = store.get(&p.id).unwrap_or_default();
    }
}

/// Moves secrets that aren't in the store yet into it, and returns the copy to write to disk
/// (secrets that made it into the store are blanked; the others stay, in the 0600 file).
fn stash_secrets(s: &mut Settings, store: &dyn SecretStore) -> Settings {
    for p in s.profiles.iter_mut().filter(|p| !p.keychain && !p.secret_key.is_empty()) {
        p.keychain = store.set(&p.id, &p.secret_key);
    }
    let mut disk = s.clone();
    for p in disk.profiles.iter_mut().filter(|p| p.keychain) {
        p.secret_key.clear();
    }
    disk
}

/// A profile from the UI comes without its secret (see [`redacted`]): an empty secret means
/// "keep the saved one". A new secret is marked for the store.
fn keep_secret(s: &Settings, p: &mut Profile) {
    let old = s.profiles.iter().find(|x| !p.id.is_empty() && x.id == p.id);
    if p.secret_key.is_empty() {
        if let Some(old) = old {
            p.secret_key = old.secret_key.clone();
            p.keychain = old.keychain;
        }
    } else {
        p.keychain = old.is_some_and(|o| o.keychain && o.secret_key == p.secret_key);
    }
}

fn write_settings(app: &AppHandle, s: &mut Settings) -> Result<(), String> {
    let disk = stash_secrets(s, &OsStore::new(app));
    let p = config_path(app)?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, serde_json::to_vec_pretty(&disk).unwrap()).map_err(|e| e.to_string())?;
    // The file may still hold secrets (no OS store available): keep it private to the user.
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

// ---------- commands ----------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    redacted(&state.settings())
}

/// Containers (buckets) visible with these credentials.
pub async fn check_profile(profile: &Profile) -> Result<Vec<String>, String> {
    Ok(Remote::new(profile).containers().await?.into_iter().map(|b| b.name).collect())
}

/// Checks a connection from the account form (an empty secret = the saved one).
#[tauri::command]
pub async fn test_profile(state: State<'_, AppState>, mut profile: Profile) -> Result<Vec<String>, String> {
    keep_secret(&state.settings(), &mut profile);
    check_profile(&profile).await
}

fn commit(app: &AppHandle, state: &AppState, mut s: Settings) -> Result<Settings, String> {
    write_settings(app, &mut s)?;
    state.set_settings(s.clone());
    crate::tray::set_accounts(app, &s);
    Ok(redacted(&s))
}

/// Adds or updates an account and makes it the active one.
#[tauri::command]
pub fn save_profile(app: AppHandle, state: State<'_, AppState>, mut profile: Profile) -> Result<Settings, String> {
    let mut s = state.settings();
    keep_secret(&s, &mut profile);
    s.active = s.upsert(profile);
    commit(&app, &state, s)
}

/// Saves an account that arrives with a fresh secret (e.g. from a sign-in flow) and makes it active.
pub fn store_account(app: &AppHandle, state: &AppState, mut p: Profile) -> Result<Settings, String> {
    let mut s = state.settings();
    p.keychain = false; // a new secret: store it again
    s.active = s.upsert(p);
    commit(app, state, s)
}

#[tauri::command]
pub fn delete_profile(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<Settings, String> {
    let mut s = state.settings();
    s.remove(&id);
    let store = OsStore::new(&app);
    store.delete(&id);
    store.delete(&crate::box_api::refresh_key(&id)); // Box sign-in, if any
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
    crate::resume::set_speed_limit(if general.experimental.resume { general.speed_limit_kbps } else { 0 });
    s.general = general;
    commit(&app, &state, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// In-memory store; `ok = false` behaves like a machine without a credential store.
    struct MemStore {
        ok: bool,
        map: RefCell<HashMap<String, String>>,
    }

    impl SecretStore for MemStore {
        fn get(&self, id: &str) -> Option<String> {
            self.map.borrow().get(id).cloned()
        }
        fn set(&self, id: &str, secret: &str) -> bool {
            if self.ok {
                self.map.borrow_mut().insert(id.into(), secret.into());
            }
            self.ok
        }
        fn delete(&self, id: &str) {
            self.map.borrow_mut().remove(id);
        }
    }

    fn mem(ok: bool) -> MemStore {
        MemStore { ok, map: RefCell::new(HashMap::new()) }
    }

    #[test]
    fn secrets_move_to_the_store_and_stay_out_of_the_file_and_ui() {
        // A pre-0.4 file: the secret is in config.json.
        let mut s = parse_settings(br#"{"profiles":[{"id":"a","endpoint":"http://x","secret_key":"S"}],"active":"a"}"#)
            .unwrap();
        let store = mem(true);
        let disk = stash_secrets(&mut s, &store);
        assert!(s.profiles[0].keychain && s.profiles[0].secret_key == "S", "memory keeps the secret");
        assert_eq!(store.get("a").as_deref(), Some("S"));
        let json = String::from_utf8(serde_json::to_vec(&disk).unwrap()).unwrap();
        assert!(!json.contains("\"S\"") && json.contains("\"keychain\":true"), "{json}");
        assert!(redacted(&s).profiles[0].secret_key.is_empty());
        // Next launch reads it back from the store.
        let mut again = parse_settings(json.as_bytes()).unwrap();
        load_secrets(&mut again, &store);
        assert_eq!(again.profiles[0].secret_key, "S");
    }

    #[test]
    fn without_a_store_the_secret_stays_in_the_file() {
        let mut s = Settings::default();
        s.upsert(Profile { id: "a".into(), secret_key: "S".into(), ..Default::default() });
        let disk = stash_secrets(&mut s, &mem(false));
        assert!(!disk.profiles[0].keychain);
        assert_eq!(disk.profiles[0].secret_key, "S");
    }

    #[test]
    fn empty_secret_from_the_form_keeps_the_saved_one() {
        let mut s = Settings::default();
        s.upsert(Profile { id: "a".into(), secret_key: "S".into(), keychain: true, ..Default::default() });
        let mut edit = Profile { id: "a".into(), name: "renamed".into(), ..Default::default() };
        keep_secret(&s, &mut edit);
        assert_eq!((edit.secret_key.as_str(), edit.keychain), ("S", true));
        let mut changed = Profile { id: "a".into(), secret_key: "T".into(), ..Default::default() };
        keep_secret(&s, &mut changed);
        assert_eq!((changed.secret_key.as_str(), changed.keychain), ("T", false), "a new secret must be stored again");
        let mut new = Profile::default();
        keep_secret(&s, &mut new);
        assert!(new.secret_key.is_empty(), "a new account never inherits another's secret");
    }

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
        assert!(
            s.general.run_in_background && s.general.check_updates && s.general.auto_sizes,
            "general defaults to on"
        );
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
