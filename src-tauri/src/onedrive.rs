//! OneDrive provider through Microsoft Graph (ADR-045). Personal, work and school accounts.
//!
//! - Sign-in: NoExplorer's own Entra app (a public client: its ID is not a secret, and there is no
//!   client secret). Authorization code + PKCE through the browser, redirected to a loopback port.
//!   The refresh token goes to the keychain (`onedrive-refresh:<profile id>`) and is saved again
//!   whenever Microsoft sends a new one; the access token lives in memory.
//! - Graph addresses items by path (`/me/drive/root:/a/b:`), so unlike Box there's no ID cache.
//! - Uploads: ≤ 4 MB in one PUT; larger files through an upload session in 50 MiB chunks (a
//!   multiple of 320 KiB, as Graph requires; it accepts up to 60 MiB). Graph takes a session's
//!   chunks strictly in order, one at a time, so big chunks are what keeps an upload moving: each
//!   chunk is a round trip plus Microsoft's own processing before the next may start. An existing
//!   file is replaced (a new version on OneDrive), like overwriting on S3.
//! - Downloads: files ≥ 32 MiB are fetched as 4 ranges at once from the item's pre-authorized
//!   download URL (one connection to Microsoft's download servers is often capped well below the
//!   line speed); smaller ones, and resumed ones, follow Graph's redirect in one stream (reqwest
//!   drops the bearer token on the cross-host hop).
//! - Deletes go to the OneDrive recycle bin.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use futures::{StreamExt, TryStreamExt};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::box_api::{enc, form, parse_rfc3339, rfc3339_in, wait_for_code};
use crate::config::Profile;
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry, drop_part, open_part, part_path, resume_offset};
use crate::secrets::{OsStore, SecretStore};
use crate::webdav::{file_stream, http};

/// NoExplorer's app registration in Microsoft Entra (public client, multi-tenant + personal).
pub const CLIENT_ID: &str = "87fa48bc-edbd-4d05-b935-ded940f85614";
const SCOPES: &str = "Files.ReadWrite offline_access User.Read";
/// Files up to this go in one PUT (Graph's limit for simple uploads).
const SIMPLE_MAX: u64 = 4 * 1024 * 1024;
/// Upload session chunk: 160 × 320 KiB = 50 MiB (Graph's limit is 60 MiB per request).
const CHUNK: u64 = 160 * 320 * 1024;
/// Files at least this big download in parallel ranges.
const PARALLEL_MIN: u64 = 32 * 1024 * 1024;
const RANGE: u64 = 16 * 1024 * 1024;
const DOWNLOAD_STREAMS: usize = 4;
const WALK_CONCURRENCY: usize = 4;
/// Path segments keep these as they are; everything else is percent-encoded.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

/// Keychain account for a profile's OneDrive refresh token.
pub fn refresh_key(profile_id: &str) -> String {
    format!("onedrive-refresh:{profile_id}")
}

struct Tokens {
    access: String,
    expires: Instant,
    refresh: String,
}

/// Graph and sign-in endpoints (overridable for the mock server in tests).
#[derive(Clone)]
pub struct Endpoints {
    /// e.g. `https://graph.microsoft.com/v1.0`
    pub graph: String,
    /// e.g. `https://login.microsoftonline.com/common/oauth2/v2.0`
    pub login: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            graph: "https://graph.microsoft.com/v1.0".into(),
            login: "https://login.microsoftonline.com/common/oauth2/v2.0".into(),
        }
    }
}

pub struct OneDrive {
    profile: Profile,
    http: reqwest::Client,
    ep: Endpoints,
    tokens: tokio::sync::Mutex<Tokens>,
    /// Persists a new refresh token.
    save_refresh: Arc<dyn Fn(&str) + Send + Sync>,
    /// The drive's id (needed to copy), fetched once.
    drive: Mutex<Option<String>>,
}

impl OneDrive {
    /// An account whose refresh token is in the keychain.
    pub fn new(p: &Profile) -> OneDrive {
        let key = refresh_key(&p.id);
        let refresh = OsStore::global().and_then(|s| s.get(&key)).unwrap_or_default();
        let save = move |t: &str| {
            if let Some(s) = OsStore::global() {
                s.set(&key, t);
            }
        };
        OneDrive::with(p, refresh, Arc::new(save), Endpoints::default())
    }

    pub fn with(p: &Profile, refresh: String, save_refresh: Arc<dyn Fn(&str) + Send + Sync>, ep: Endpoints) -> Self {
        OneDrive {
            profile: p.clone(),
            http: http(),
            ep,
            tokens: tokio::sync::Mutex::new(Tokens { access: String::new(), expires: Instant::now(), refresh }),
            save_refresh,
            drive: Mutex::new(None),
        }
    }

    pub fn account(&self) -> &str {
        &self.profile.id
    }

    // ---------- auth + requests ----------

    /// A valid access token, refreshing (and saving a new refresh token) when needed.
    async fn access(&self, force: bool) -> Result<String, String> {
        let mut t = self.tokens.lock().await;
        if !force && !t.access.is_empty() && Instant::now() < t.expires {
            return Ok(t.access.clone());
        }
        if t.refresh.is_empty() {
            return Err("Not signed in to OneDrive. Open Settings → Accounts and sign in again.".into());
        }
        let r = self
            .http
            .post(format!("{}/token", self.ep.login))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form(&[
                ("client_id", CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", &t.refresh),
                ("scope", SCOPES),
            ]))
            .send()
            .await
            .map_err(net_err)?;
        if r.status() == StatusCode::BAD_REQUEST || r.status() == StatusCode::UNAUTHORIZED {
            return Err("OneDrive sign-in expired. Open Settings → Accounts and sign in again.".into());
        }
        let v: Value = ok(r).await?.json().await.map_err(net_err)?;
        let Some(access) = v["access_token"].as_str() else {
            return Err("Microsoft didn't return a token".into());
        };
        t.access = access.to_string();
        t.expires = Instant::now() + Duration::from_secs(v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(120));
        if let Some(refresh) = v["refresh_token"].as_str().filter(|r| *r != t.refresh) {
            t.refresh = refresh.to_string();
            (self.save_refresh)(refresh);
        }
        Ok(t.access.clone())
    }

    /// Sends a request built by `build` with auth; refreshes once on 401, waits on 429/503.
    async fn send(&self, build: impl Fn(&reqwest::Client) -> RequestBuilder) -> Result<Response, String> {
        let mut force = false;
        for attempt in 0..4 {
            let token = self.access(force).await?;
            let r = build(&self.http).bearer_auth(token).send().await.map_err(net_err)?;
            match r.status() {
                StatusCode::UNAUTHORIZED if !force => force = true,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE if attempt < 3 => {
                    tokio::time::sleep(retry_after(&r)).await;
                }
                _ => return Ok(r),
            }
        }
        Err("OneDrive is busy (too many requests). Try again in a minute.".into())
    }

    async fn json(&self, build: impl Fn(&reqwest::Client) -> RequestBuilder) -> Result<Value, String> {
        ok(self.send(build).await?).await?.json().await.map_err(net_err)
    }

    /// The Graph URL of the item at `key` (folders may end with `/`; "" = the drive's root).
    fn item(&self, key: &str) -> String {
        let p = key.trim_matches('/');
        if p.is_empty() {
            return format!("{}/me/drive/root", self.ep.graph);
        }
        let path: Vec<String> =
            p.split('/').map(|s| percent_encoding::utf8_percent_encode(s, SEGMENT).to_string()).collect();
        format!("{}/me/drive/root:/{}:", self.ep.graph, path.join("/"))
    }

    fn by_id(&self, id: &str) -> String {
        format!("{}/me/drive/items/{}", self.ep.graph, enc(id))
    }

    /// The item's id, or None when nothing is there.
    async fn id_of(&self, key: &str) -> Result<Option<String>, String> {
        let url = format!("{}?$select=id", self.item(key));
        let r = self.send(|h| h.get(&url)).await?;
        if r.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let v: Value = ok(r).await?.json().await.map_err(net_err)?;
        Ok(v["id"].as_str().map(String::from))
    }

    /// Folder `path` (ending with `/`, "" = root), created with its parents when missing. Returns its id.
    async fn ensure_folder(&self, path: &str) -> Result<String, String> {
        if let Some(id) = self.id_of(path).await? {
            return Ok(id);
        }
        let mut cur = String::new();
        let mut parent = self.id_of("").await?.ok_or("OneDrive has no root folder")?;
        for seg in path.trim_end_matches('/').split('/').filter(|s| !s.is_empty()) {
            cur = format!("{cur}{seg}/");
            parent = match self.id_of(&cur).await? {
                Some(id) => id,
                None => self.create_folder(&parent, seg, &cur).await?,
            };
        }
        Ok(parent)
    }

    async fn create_folder(&self, parent_id: &str, name: &str, key: &str) -> Result<String, String> {
        let url = format!("{}/children", self.by_id(parent_id));
        let body = json!({ "name": name, "folder": {}, "@microsoft.graph.conflictBehavior": "fail" });
        let r = self.send(|h| h.post(&url).json(&body)).await?;
        if r.status() == StatusCode::CONFLICT {
            // Made meanwhile (a parallel upload): use it.
            return self.id_of(key).await?.ok_or_else(|| "Folder exists".into());
        }
        let v: Value = ok(r).await?.json().await.map_err(net_err)?;
        v["id"].as_str().map(String::from).ok_or_else(|| "OneDrive didn't return the folder id".into())
    }

    async fn drive_id(&self) -> Result<String, String> {
        if let Some(d) = self.drive.lock().unwrap().clone() {
            return Ok(d);
        }
        self.containers().await?;
        self.drive.lock().unwrap().clone().ok_or_else(|| "OneDrive didn't return the drive id".into())
    }

    // ---------- provider methods ----------

    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        let url = format!("{}/me/drive?$select=id,driveType", self.ep.graph);
        let v = self.json(|h| h.get(&url)).await?; // doubles as the connection test
        *self.drive.lock().unwrap() = v["id"].as_str().map(String::from);
        Ok(vec![BucketInfo { name: "OneDrive".into(), region: None }])
    }

    pub async fn list(&self, _container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let folder = if prefix.is_empty() || prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
        let mut url = format!(
            "{}/children?$select=name,size,folder,file,package,remoteItem,lastModifiedDateTime",
            self.item(&folder)
        );
        let mut out = vec![];
        loop {
            let v = self.json(|h| h.get(&url)).await?;
            for e in v["value"].as_array().into_iter().flatten() {
                // OneNote notebooks (packages) and items shared from other drives can't be
                // addressed by path in this drive.
                if !e["package"].is_null() || !e["remoteItem"].is_null() {
                    continue;
                }
                let Some(name) = e["name"].as_str() else { continue };
                let dir = e["folder"].is_object();
                out.push(Entry {
                    key: if dir { format!("{folder}{name}/") } else { format!("{folder}{name}") },
                    name: name.to_string(),
                    size: if dir { 0 } else { e["size"].as_u64().unwrap_or(0) },
                    modified: e["lastModifiedDateTime"].as_str().and_then(parse_rfc3339),
                    dir,
                    // Personal accounts report SHA-1; work accounts only quickXorHash (not used).
                    hash: e["file"]["hashes"]["sha1Hash"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("sha1:{}", s.to_ascii_lowercase())),
                });
            }
            match v["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_string(),
                None => return Ok(out),
            }
        }
    }

    pub async fn list_all(&self, container: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        if !prefix.is_empty() && !prefix.ends_with('/') {
            let l = self.list(container, &parent_of(prefix)).await?;
            return Ok(l.into_iter().filter(|e| e.key == prefix).map(|e| (e.key, e.size)).collect());
        }
        let mut out = vec![];
        let mut level = vec![prefix.to_string()];
        while !level.is_empty() {
            let lists: Vec<Result<Vec<Entry>, String>> = futures::stream::iter(level.drain(..))
                .map(|dir| async move { self.list(container, &dir).await })
                .buffer_unordered(WALK_CONCURRENCY)
                .collect()
                .await;
            for l in lists {
                for e in l? {
                    if e.dir {
                        level.push(e.key);
                    } else {
                        out.push((e.key, e.size));
                    }
                }
            }
        }
        Ok(out)
    }

    /// `inline`: a preview URL through the local proxy; otherwise an anonymous view link. OneDrive
    /// only lets some plans set an expiry, so without one the link is made without it.
    pub async fn link(&self, container: &str, key: &str, secs: u64, inline: bool) -> Result<String, String> {
        if inline {
            return Ok(crate::proxy::url(self.account(), container, key));
        }
        let url = format!("{}/createLink", self.item(key));
        let timed = json!({ "type": "view", "scope": "anonymous", "expirationDateTime": rfc3339_in(secs.max(3600)) });
        let r = self.send(|h| h.post(&url).json(&timed)).await?;
        let r = if r.status().is_client_error() {
            let plain = json!({ "type": "view", "scope": "anonymous" });
            self.send(|h| h.post(&url).json(&plain)).await?
        } else {
            r
        };
        let v: Value = ok(r).await?.json().await.map_err(net_err)?;
        v["link"]["webUrl"].as_str().map(String::from).ok_or_else(|| "OneDrive didn't return a link".into())
    }

    pub async fn read_range(
        &self,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        let last = end.unwrap_or(start + cap - 1).min(start + cap - 1);
        let url = format!("{}/content", self.item(key));
        let r = ok(self.send(|h| h.get(&url).header("Range", format!("bytes={start}-{last}"))).await?).await?;
        let total = r
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|n| n.parse().ok())
            .or_else(|| (r.status() == StatusCode::OK).then(|| r.content_length()).flatten());
        let mut bytes = r.bytes().await.map_err(net_err)?.to_vec();
        bytes.truncate(cap as usize);
        Ok((bytes, total))
    }

    pub async fn read_head(&self, _container: &str, key: &str, max: u64) -> Result<Vec<u8>, String> {
        Ok(self.read_range(key, 0, None, max.max(1)).await?.0)
    }

    pub async fn mkdir(&self, _container: &str, prefix: &str, name: &str) -> Result<(), String> {
        let parent = self.ensure_folder(prefix).await?;
        let url = format!("{}/children", self.by_id(&parent));
        let body = json!({ "name": name, "folder": {}, "@microsoft.graph.conflictBehavior": "fail" });
        ok(self.send(|h| h.post(&url).json(&body)).await?).await.map(|_| ())
    }

    /// Copies or moves (`keep` = copy) `from` to `to`. Never overwrites: a taken name is an error.
    /// Copies run on Microsoft's side; NoExplorer follows the monitor URL until they finish.
    pub async fn transfer_within(
        &self,
        _container: &str,
        from: &str,
        to: &str,
        keep: bool,
        job: Option<&Job>,
    ) -> Result<(), String> {
        if let Some(j) = job {
            j.files_total.store(1, Ordering::Relaxed);
        }
        let to_name = to.trim_end_matches('/');
        let (parent_path, name) = match to_name.rfind('/') {
            Some(i) => (&to_name[..=i], &to_name[i + 1..]),
            None => ("", to_name),
        };
        if self.id_of(to).await?.is_some() {
            return Err(format!("“{name}” already exists there"));
        }
        let id = self.id_of(from).await?.ok_or_else(|| format!("Not found: {}", from.trim_end_matches('/')))?;
        let parent = self.ensure_folder(parent_path).await?;
        if keep {
            let drive = self.drive_id().await?;
            let url = format!("{}/copy?@microsoft.graph.conflictBehavior=fail", self.by_id(&id));
            let body = json!({ "parentReference": { "driveId": drive, "id": parent }, "name": name });
            let r = ok(self.send(|h| h.post(&url).json(&body)).await?).await?;
            let monitor = r.headers().get("location").and_then(|v| v.to_str().ok()).map(String::from);
            if let Some(m) = monitor {
                self.wait_copy(&m, job).await?;
            }
        } else {
            let body = json!({ "name": name, "parentReference": { "id": parent },
                               "@microsoft.graph.conflictBehavior": "fail" });
            let url = self.by_id(&id);
            let r = self.send(|h| h.patch(&url).json(&body)).await?;
            if r.status() == StatusCode::CONFLICT {
                return Err(format!("“{name}” already exists there"));
            }
            ok(r).await?;
        }
        if let Some(j) = job {
            j.files_done.store(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Polls a copy's monitor URL (pre-authorized: no token) until it's done.
    async fn wait_copy(&self, monitor: &str, job: Option<&Job>) -> Result<(), String> {
        // The finished monitor redirects to the new item, which needs a token: stop at the redirect.
        let poll = crate::webdav::http_builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(60 * 60);
        while Instant::now() < deadline {
            if job.is_some_and(|j| j.cancelled()) {
                return Err("cancelled".into());
            }
            let r = poll.get(monitor).send().await.map_err(net_err)?;
            if r.status().is_redirection() {
                return Ok(());
            }
            let v: Value = r.json().await.unwrap_or(Value::Null);
            match v["status"].as_str() {
                Some("completed") => return Ok(()),
                Some("failed") => {
                    let why = v["error"]["message"].as_str().unwrap_or("the copy failed");
                    return Err(format!("OneDrive: {why}"));
                }
                _ => tokio::time::sleep(Duration::from_millis(800)).await,
            }
        }
        Err("OneDrive took too long to copy".into())
    }

    /// Moves files and folders (with everything inside) to the OneDrive recycle bin.
    pub async fn delete(&self, _container: &str, keys: Vec<String>, job: Option<&Job>) -> Result<usize, String> {
        let mut n = 0;
        let total = keys.len() as u64;
        if let Some(j) = job {
            j.files_total.store(total, Ordering::Relaxed);
        }
        for (i, k) in keys.into_iter().enumerate() {
            if let Some(j) = job {
                if j.cancelled() {
                    return Err("cancelled".into());
                }
                j.files_done.store(i as u64, Ordering::Relaxed);
            }
            let url = self.item(&k);
            let r = self.send(|h| h.delete(&url)).await?;
            if r.status() != StatusCode::NOT_FOUND {
                ok(r).await?;
                n += 1;
            }
        }
        if let Some(j) = job {
            j.files_done.store(total, Ordering::Relaxed);
        }
        Ok(n)
    }

    pub async fn upload_file(
        &self,
        _container: &str,
        key: &str,
        path: &Path,
        size: u64,
        job: &Job,
    ) -> Result<(), String> {
        if size <= SIMPLE_MAX {
            // Graph creates missing parent folders for a path PUT.
            let url = format!("{}/content?@microsoft.graph.conflictBehavior=replace", self.item(key));
            let r = self
                .send(|h| {
                    h.put(&url)
                        .header("Content-Type", "application/octet-stream")
                        .header("Content-Length", size)
                        .body(reqwest::Body::wrap_stream(file_stream(path.to_path_buf(), 0, size, job)))
                })
                .await?;
            return ok(r).await.map(|_| ());
        }
        let url = format!("{}/createUploadSession", self.item(key));
        let body = json!({ "item": { "@microsoft.graph.conflictBehavior": "replace" } });
        let s = self.json(|h| h.post(&url).json(&body)).await?;
        let upload = s["uploadUrl"].as_str().ok_or("OneDrive didn't open an upload session")?.to_string();
        let res: Result<(), String> = async {
            let mut offset = 0;
            while offset < size {
                if job.cancelled() {
                    return Err("cancelled".into());
                }
                let n = CHUNK.min(size - offset);
                // The upload URL carries its own authorization: no bearer token here.
                let r = self
                    .http
                    .put(&upload)
                    .header("Content-Length", n)
                    .header("Content-Range", format!("bytes {offset}-{}/{size}", offset + n - 1))
                    .body(reqwest::Body::wrap_stream(file_stream(path.to_path_buf(), offset, n, job)))
                    .send()
                    .await
                    .map_err(net_err)?;
                ok(r).await?;
                offset += n;
            }
            Ok(())
        }
        .await;
        if res.is_err() {
            let _ = self.http.delete(&upload).send().await;
        }
        res
    }

    /// Streams a file to `path` via `path.part`, renaming on success.
    pub async fn download_file(&self, _container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let tmp = part_path(path);
        let offset = resume_offset(&tmp, job).await;
        if offset == 0 {
            let meta = self.json(|h| h.get(self.item(key))).await?;
            let size = meta["size"].as_u64().unwrap_or(0);
            if let (true, Some(dl)) = (size >= PARALLEL_MIN, meta["@microsoft.graph.downloadUrl"].as_str()) {
                match self.download_ranges(dl, size, &tmp, job).await {
                    Ok(()) => return tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string()),
                    // The server ignored Range: fall back to one stream below.
                    Err(e) if e == NO_RANGES => job.bytes_done.store(0, Ordering::Relaxed),
                    Err(e) => return Err(e),
                }
            }
        }
        let url = format!("{}/content", self.item(key));
        let r = self
            .send(|h| {
                let b = h.get(&url);
                if offset > 0 { b.header("Range", format!("bytes={offset}-")) } else { b }
            })
            .await?;
        if offset > 0 && r.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            job.bytes_done.fetch_add(offset, Ordering::Relaxed);
            return tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string());
        }
        let partial = offset > 0 && r.status() == StatusCode::PARTIAL_CONTENT;
        let r = ok(r).await?;
        let mut body = r.bytes_stream();
        let f = open_part(&tmp, partial, offset, job).await?;
        let mut w = tokio::io::BufWriter::with_capacity(1 << 20, f);
        let res: Result<(), String> = async {
            while let Some(chunk) = body.try_next().await.map_err(net_err)? {
                if job.cancelled() {
                    return Err("cancelled".into());
                }
                w.write_all(&chunk).await.map_err(|e| e.to_string())?;
                job.flow(chunk.len() as u64).await;
            }
            w.flush().await.map_err(|e| e.to_string())
        }
        .await;
        drop(w);
        match res {
            Ok(()) => tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string()),
            Err(e) => {
                drop_part(&tmp, job, &e).await;
                Err(e)
            }
        }
    }
}

const NO_RANGES: &str = "no ranges";

impl OneDrive {
    /// Downloads `size` bytes from a pre-authorized URL as [`DOWNLOAD_STREAMS`] ranges at once,
    /// each written at its offset in `tmp`. A failure removes `tmp`: with gaps in it, it can't be
    /// resumed from its length the way a one-stream `.part` can.
    async fn download_ranges(&self, url: &str, size: u64, tmp: &Path, job: &Job) -> Result<(), String> {
        let f = tokio::fs::File::create(tmp).await.map_err(|e| e.to_string())?;
        f.set_len(size).await.map_err(|e| e.to_string())?;
        drop(f);
        let ranges: Vec<(u64, u64)> =
            (0..size).step_by(RANGE as usize).map(|a| (a, (a + RANGE).min(size) - 1)).collect();
        let res: Result<Vec<()>, String> = futures::stream::iter(ranges)
            .map(|(a, b)| async move {
                // The download URL carries its own authorization: no bearer token.
                let r = self.http.get(url).header("Range", format!("bytes={a}-{b}")).send().await.map_err(net_err)?;
                if r.status() != StatusCode::PARTIAL_CONTENT {
                    return Err(if r.status().is_success() {
                        NO_RANGES.to_string()
                    } else {
                        ok(r).await.err().unwrap_or_default()
                    });
                }
                let mut f = tokio::fs::OpenOptions::new().write(true).open(tmp).await.map_err(|e| e.to_string())?;
                tokio::io::AsyncSeekExt::seek(&mut f, std::io::SeekFrom::Start(a)).await.map_err(|e| e.to_string())?;
                let mut w = tokio::io::BufWriter::with_capacity(1 << 20, f);
                let mut body = r.bytes_stream();
                let mut got = 0u64;
                while let Some(chunk) = body.try_next().await.map_err(net_err)? {
                    if job.cancelled() {
                        return Err("cancelled".to_string());
                    }
                    w.write_all(&chunk).await.map_err(|e| e.to_string())?;
                    got += chunk.len() as u64;
                    job.flow(chunk.len() as u64).await;
                }
                w.flush().await.map_err(|e| e.to_string())?;
                if got != b - a + 1 {
                    return Err("OneDrive ended a download early".to_string());
                }
                Ok(())
            })
            .buffer_unordered(DOWNLOAD_STREAMS)
            .try_collect()
            .await;
        if let Err(e) = res {
            let _ = tokio::fs::remove_file(tmp).await;
            return Err(e);
        }
        Ok(())
    }
}

// ---------- sign-in ----------

/// PKCE: a random verifier and its S256 challenge.
fn pkce() -> (String, String) {
    let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Signs in to OneDrive through the browser and saves the account (refresh token in the keychain).
/// `id` = an existing account to sign in again. Returns the settings.
#[tauri::command]
pub async fn onedrive_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
    id: Option<String>,
    name: String,
) -> Result<crate::config::Settings, String> {
    use tauri_plugin_opener::OpenerExt;
    let ep = Endpoints::default();
    let existing = id.as_deref().and_then(|id| state.settings().profiles.iter().find(|p| p.id == id).cloned());
    // Any port works: Entra ignores the port of a http://localhost redirect for desktop apps.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Can't listen for the Microsoft sign-in ({e})"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://localhost:{port}");
    crate::webdav::LOGIN_CANCEL.store(false, Ordering::Relaxed);
    let csrf = uuid::Uuid::new_v4().simple().to_string();
    let (verifier, challenge) = pkce();
    let auth = format!(
        "{}/authorize?client_id={CLIENT_ID}&response_type=code&response_mode=query&redirect_uri={}&scope={}&state={csrf}&code_challenge={challenge}&code_challenge_method=S256&prompt=select_account",
        ep.login,
        enc(&redirect),
        enc(SCOPES)
    );
    app.opener().open_url(&auth, None::<&str>).map_err(|e| e.to_string())?;
    let code = wait_for_code(&listener, &csrf, "/?", "OneDrive").await?;
    drop(listener);

    let r = http()
        .post(format!("{}/token", ep.login))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("code_verifier", &verifier),
            ("scope", SCOPES),
        ]))
        .send()
        .await
        .map_err(net_err)?;
    let v: Value = ok(r).await?.json().await.map_err(net_err)?;
    let (Some(access), Some(refresh)) = (v["access_token"].as_str(), v["refresh_token"].as_str()) else {
        return Err("Microsoft didn't return tokens".into());
    };

    let mut p = existing.unwrap_or_default();
    if p.id.is_empty() {
        p.id = uuid::Uuid::new_v4().to_string();
    }
    p.kind = crate::config::ProviderKind::Onedrive;
    p.endpoint.clear();
    p.secret_key.clear();
    let probe = OneDrive::with(&p, refresh.to_string(), Arc::new(|_: &str| {}), ep.clone());
    {
        let mut t = probe.tokens.lock().await;
        t.access = access.to_string();
        t.expires = Instant::now() + Duration::from_secs(600);
    }
    let me = probe.json(|h| h.get(format!("{}/me?$select=userPrincipalName,mail", ep.graph))).await?;
    let login = me["mail"].as_str().or(me["userPrincipalName"].as_str()).unwrap_or("").to_string();
    p.access_key = login.clone(); // shown in the account list; not a credential
    p.name = match name.trim() {
        "" if !login.is_empty() => format!("OneDrive · {login}"),
        "" => "OneDrive".into(),
        n => n.to_string(),
    };
    let store = OsStore::new(&app);
    if !store.set(&refresh_key(&p.id), refresh) {
        return Err("Couldn't save the OneDrive sign-in in the system keychain".into());
    }
    state.forget(&p.id);
    crate::config::store_account(&app, &state, p)
}

// ---------- helpers ----------

fn parent_of(key: &str) -> String {
    match key.trim_end_matches('/').rfind('/') {
        Some(i) => key[..=i].to_string(),
        None => String::new(),
    }
}

fn retry_after(r: &Response) -> Duration {
    let s = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok()).unwrap_or(2u64);
    Duration::from_secs(s.min(30))
}

fn net_err(e: reqwest::Error) -> String {
    if e.is_connect() {
        "Can't reach OneDrive (check your connection)".into()
    } else if e.is_timeout() {
        "OneDrive took too long to answer".into()
    } else {
        e.without_url().to_string()
    }
}

/// Passes 2xx through; turns Graph's JSON errors into readable messages. Logged URLs lose their
/// query: upload and download URLs carry their authorization there.
async fn ok(r: Response) -> Result<Response, String> {
    let st = r.status();
    if st.is_success() {
        return Ok(r);
    }
    let mut url = r.url().clone();
    url.set_query(None);
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if st.as_u16() != 404 {
        crate::diag::log("onedrive", &format!("HTTP {} {url} {}", st.as_u16(), v["error"]));
    }
    let msg = v["error"]["message"].as_str().or(v["error_description"].as_str()).unwrap_or("").to_string();
    let what = match st.as_u16() {
        401 => "OneDrive sign-in expired. Sign in again in Settings → Accounts",
        403 => "Not allowed on OneDrive",
        404 => "Not found on OneDrive",
        409 => "Something with that name already exists",
        413 => "File too large for OneDrive",
        507 => "Not enough space on OneDrive",
        _ => "OneDrive request failed",
    };
    Err(if msg.is_empty() {
        format!("{what} (HTTP {})", st.as_u16())
    } else {
        format!("{what} (HTTP {}): {msg}", st.as_u16())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::io::AsyncReadExt;

    // ---------- a tiny in-memory Graph, enough to drive OneDrive end to end ----------

    #[derive(Default)]
    struct Mock {
        /// Path ("a/b", folders without a trailing slash) → file bytes, or None for a folder.
        items: HashMap<String, Option<Vec<u8>>>,
        ids: HashMap<String, String>,
        /// Upload sessions: id → (path, size, bytes so far).
        sessions: HashMap<String, (String, u64, Vec<u8>)>,
        refresh: String,
        access: String,
        expire_next: bool,
        refreshes: u32,
        next: u64,
        base: String,
        /// Ranged requests to the download URL (parallel downloads).
        ranged: u32,
    }

    impl Mock {
        fn id(&mut self, path: &str) -> String {
            if let Some(i) = self.ids.get(path) {
                return i.clone();
            }
            self.next += 1;
            let id = format!("ID{}", self.next);
            self.ids.insert(path.to_string(), id.clone());
            id
        }
        fn path_of(&self, id: &str) -> Option<String> {
            self.ids
                .iter()
                .find(|(p, i)| *i == id && (p.is_empty() || self.items.contains_key(*p)))
                .map(|(p, _)| p.clone())
        }
        fn exists(&self, p: &str) -> bool {
            p.is_empty() || self.items.contains_key(p)
        }
        /// Creates `p`'s missing parent folders.
        fn parents(&mut self, p: &str) {
            let segs: Vec<&str> = p.split('/').collect();
            for i in 1..segs.len() {
                self.items.entry(segs[..i].join("/")).or_insert(None);
            }
        }
        fn json(&mut self, p: &str) -> Value {
            let id = self.id(p);
            let name = p.rsplit('/').next().unwrap_or("").to_string();
            match self.items.get(p).cloned().flatten() {
                Some(d) => {
                    json!({ "id": id.clone(), "name": name, "size": d.len(), "lastModifiedDateTime": "2026-10-20T10:00:00Z",
                            "@microsoft.graph.downloadUrl": format!("{}/dl/{id}", self.base),
                                  "file": { "hashes": { "sha1Hash": hex(&sha1::Sha1::digest(&d)).to_uppercase() } } })
                }
                None => {
                    json!({ "id": id, "name": name, "folder": { "childCount": 0 }, "lastModifiedDateTime": "2026-10-20T10:00:00Z" })
                }
            }
        }
        fn remove(&mut self, p: &str) {
            let pre = format!("{p}/");
            self.items.retain(|k, _| k != p && !k.starts_with(&pre));
        }
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn dec(s: &str) -> String {
        percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
    }

    async fn read_req(sock: &mut tokio::net::TcpStream) -> Option<(String, String, HashMap<String, String>, Vec<u8>)> {
        let mut buf = vec![];
        let mut tmp = [0u8; 65536];
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            let n = sock.read(&mut tmp).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&tmp[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split_whitespace();
        let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let mut body = buf[head_end + 4..].to_vec();
        let len = headers.get("content-length").and_then(|l| l.parse::<usize>().ok()).unwrap_or(0);
        while body.len() < len {
            let n = sock.read(&mut tmp).await.ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        Some((method, target, headers, body))
    }

    fn respond(status: u16, body: &[u8], extra: &str) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn err(status: u16, code: &str) -> Vec<u8> {
        respond(status, json!({ "error": { "code": code, "message": code } }).to_string().as_bytes(), "")
    }

    /// `/v1.0/me/drive/root:/a/b:/children` → ("a/b", "/children"); `/v1.0/me/drive/root/children` → ("", "/children").
    fn split_path(path: &str) -> Option<(String, String)> {
        let rest = path.strip_prefix("/v1.0/me/drive/root")?;
        if let Some(r) = rest.strip_prefix(":/") {
            let (p, tail) = r.split_once(':').unwrap_or((r, ""));
            return Some((dec(p), tail.to_string()));
        }
        Some((String::new(), rest.to_string()))
    }

    async fn serve(state: Arc<Mutex<Mock>>, mut sock: tokio::net::TcpStream) {
        let Some((method, target, headers, body)) = read_req(&mut sock).await else { return };
        let (path, query) = target.split_once('?').unwrap_or((&target, ""));
        let reply = {
            let mut m = state.lock().unwrap();
            let base = m.base.clone();
            if path == "/login/token" {
                let f = crate::box_api::parse_query(&String::from_utf8_lossy(&body));
                assert_eq!(f.get("client_id").map(String::as_str), Some(CLIENT_ID));
                if f.get("refresh_token") != Some(&m.refresh) {
                    err(400, "invalid_grant")
                } else {
                    m.refreshes += 1;
                    m.refresh = format!("refresh-{}", m.refreshes);
                    m.access = format!("access-{}", m.refreshes);
                    respond(
                        200,
                        json!({ "access_token": m.access, "refresh_token": m.refresh, "expires_in": 3600 })
                            .to_string()
                            .as_bytes(),
                        "",
                    )
                }
            } else if let Some(sid) = path.strip_prefix("/upload/") {
                // Pre-authorized upload URL: must NOT carry the bearer token.
                assert!(!headers.contains_key("authorization"), "token sent to the upload URL");
                let range = headers["content-range"].trim_start_matches("bytes ").to_string();
                let (a, rest) = range.split_once('-').unwrap();
                let a: usize = a.parse().unwrap();
                let s = m.sessions.get_mut(sid).unwrap();
                s.1 = rest.rsplit('/').next().unwrap().parse().unwrap();
                assert_eq!(a, s.2.len(), "chunks in order");
                assert!(
                    (body.len() as u64).is_multiple_of(320 * 1024) || (a + body.len()) as u64 == s.1,
                    "320 KiB multiple"
                );
                s.2.extend_from_slice(&body);
                if s.2.len() as u64 == s.1 {
                    let (p, _, data) = m.sessions.remove(sid).unwrap();
                    m.parents(&p);
                    m.items.insert(p.clone(), Some(data));
                    let j = m.json(&p);
                    respond(201, j.to_string().as_bytes(), "")
                } else {
                    respond(202, b"{}", "")
                }
            } else if let Some(id) = path.strip_prefix("/dl/") {
                assert!(!headers.contains_key("authorization"), "token sent to the download URL");
                let p = m.path_of(id).unwrap();
                let data = m.items[&p].clone().unwrap();
                match headers.get("range").and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.split_once('-')) {
                    Some((a, b)) => {
                        m.ranged += 1;
                        let a: usize = a.parse().unwrap();
                        let b = b.parse::<usize>().unwrap_or(data.len() - 1).min(data.len() - 1);
                        respond(206, &data[a..=b], &format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()))
                    }
                    None => respond(200, &data, ""),
                }
            } else if path.starts_with("/monitor/") {
                respond(200, br#"{"status":"completed"}"#, "")
            } else if headers.get("authorization") != Some(&format!("Bearer {}", m.access)) || m.expire_next {
                m.expire_next = false;
                err(401, "InvalidAuthenticationToken")
            } else if path == "/v1.0/me/drive" {
                respond(200, br#"{"id":"DRIVE1","driveType":"personal"}"#, "")
            } else if let Some(id) = path.strip_prefix("/v1.0/me/drive/items/") {
                let (id, tail) = id.split_once('/').unwrap_or((id, ""));
                let id = dec(id);
                let Some(src) = m.path_of(&id) else { return };
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                match (method.as_str(), tail) {
                    ("POST", "children") => {
                        let name = v["name"].as_str().unwrap();
                        let p = if src.is_empty() { name.to_string() } else { format!("{src}/{name}") };
                        if m.exists(&p) {
                            err(409, "nameAlreadyExists")
                        } else {
                            m.items.insert(p.clone(), None);
                            let j = m.json(&p);
                            respond(201, j.to_string().as_bytes(), "")
                        }
                    }
                    ("PATCH", "") => {
                        let parent = m.path_of(v["parentReference"]["id"].as_str().unwrap()).unwrap();
                        let name = v["name"].as_str().unwrap();
                        let p = if parent.is_empty() { name.to_string() } else { format!("{parent}/{name}") };
                        if m.exists(&p) {
                            err(409, "nameAlreadyExists")
                        } else {
                            let pre = format!("{src}/");
                            let moved: Vec<(String, Option<Vec<u8>>)> = m
                                .items
                                .iter()
                                .filter(|(k, _)| **k == src || k.starts_with(&pre))
                                .map(|(k, d)| (format!("{p}{}", &k[src.len()..]), d.clone()))
                                .collect();
                            m.remove(&src);
                            m.items.extend(moved);
                            let j = m.json(&p);
                            respond(200, j.to_string().as_bytes(), "")
                        }
                    }
                    ("POST", "copy") => {
                        assert_eq!(v["parentReference"]["driveId"], "DRIVE1");
                        let parent = m.path_of(v["parentReference"]["id"].as_str().unwrap()).unwrap();
                        let name = v["name"].as_str().unwrap();
                        let p = if parent.is_empty() { name.to_string() } else { format!("{parent}/{name}") };
                        let pre = format!("{src}/");
                        let copies: Vec<(String, Option<Vec<u8>>)> = m
                            .items
                            .iter()
                            .filter(|(k, _)| **k == src || k.starts_with(&pre))
                            .map(|(k, d)| (format!("{p}{}", &k[src.len()..]), d.clone()))
                            .collect();
                        m.items.extend(copies);
                        respond(202, b"", &format!("Location: {base}/monitor/1\r\n"))
                    }
                    _ => err(404, "itemNotFound"),
                }
            } else if let Some((p, tail)) = split_path(path) {
                match (method.as_str(), tail.as_str()) {
                    ("GET", "") if m.exists(&p) => {
                        let j = m.json(&p);
                        respond(200, j.to_string().as_bytes(), "")
                    }
                    ("GET", "/children") if m.exists(&p) => {
                        let pre = if p.is_empty() { String::new() } else { format!("{p}/") };
                        let mut kids: Vec<String> = m
                            .items
                            .keys()
                            .filter(|k| k.starts_with(&pre) && !k[pre.len()..].contains('/'))
                            .cloned()
                            .collect();
                        kids.sort();
                        // Two per page: test paging through @odata.nextLink.
                        let skip: usize =
                            crate::box_api::parse_query(query).get("skip").and_then(|s| s.parse().ok()).unwrap_or(0);
                        let page: Vec<Value> = kids
                            .iter()
                            .skip(skip)
                            .take(2)
                            .cloned()
                            .collect::<Vec<_>>()
                            .iter()
                            .map(|k| m.json(k))
                            .collect();
                        let mut v = json!({ "value": page });
                        if skip + 2 < kids.len() {
                            v["@odata.nextLink"] = json!(format!("{base}{path}?skip={}", skip + 2));
                        }
                        respond(200, v.to_string().as_bytes(), "")
                    }
                    ("GET", "/content") if m.items.get(&p).is_some_and(|d| d.is_some()) => {
                        // Graph redirects to a pre-authorized download URL.
                        respond(302, b"", &format!("Location: {base}/dl/{}\r\n", m.id(&p)))
                    }
                    ("PUT", "/content") => {
                        m.parents(&p);
                        m.items.insert(p.clone(), Some(body.clone()));
                        let j = m.json(&p);
                        respond(201, j.to_string().as_bytes(), "")
                    }
                    ("POST", "/createUploadSession") => {
                        m.next += 1;
                        let sid = format!("s{}", m.next);
                        m.sessions.insert(sid.clone(), (p, 0, vec![]));
                        respond(200, json!({ "uploadUrl": format!("{base}/upload/{sid}") }).to_string().as_bytes(), "")
                    }
                    ("POST", "/createLink") => respond(
                        200,
                        json!({ "link": { "webUrl": format!("https://1drv.ms/x/{}", m.id(&p)) } })
                            .to_string()
                            .as_bytes(),
                        "",
                    ),
                    ("DELETE", "") if m.exists(&p) => {
                        m.remove(&p);
                        respond(204, b"", "")
                    }
                    _ => err(404, "itemNotFound"),
                }
            } else {
                err(404, "noRoute")
            }
        };
        let _ = sock.write_all(&reply).await;
    }

    #[tokio::test]
    async fn onedrive_roundtrip_against_mock() {
        let state = Arc::new(Mutex::new(Mock { refresh: "refresh-0".into(), ..Default::default() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        // A different host name than the API's, so reqwest treats the download redirect as
        // cross-host and drops the token (as it does for Microsoft's real download URLs).
        let addr = listener.local_addr().unwrap();
        state.lock().unwrap().base = format!("http://localhost:{}", addr.port());
        let st = state.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                tokio::spawn(serve(st.clone(), sock));
            }
        });
        let saved = Arc::new(Mutex::new(String::new()));
        let saved2 = saved.clone();
        let p = Profile { id: "od".into(), ..Default::default() };
        let ep = Endpoints { graph: format!("http://{addr}/v1.0"), login: format!("http://{addr}/login") };
        let d = OneDrive::with(
            &p,
            "refresh-0".into(),
            Arc::new(move |t: &str| *saved2.lock().unwrap() = t.to_string()),
            ep.clone(),
        );

        assert_eq!(d.containers().await.unwrap()[0].name, "OneDrive");
        assert_eq!(*saved.lock().unwrap(), "refresh-1", "new refresh token saved");

        let dir = std::env::temp_dir().join(format!("nx-onedrive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..(55 * 1024 * 1024 + 7)).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(dir.join("big.bin"), &big).unwrap();
        std::fs::write(dir.join("a.txt"), "hello onedrive").unwrap();
        let job = Job::untracked("up", "t".into(), "od", "OneDrive", "");
        d.upload_file("", "docs/sub/a b#1.txt", &dir.join("a.txt"), 14, &job).await.unwrap(); // parents made by Graph
        let size = big.len() as u64;
        d.upload_file("", "docs/big.bin", &dir.join("big.bin"), size, &job).await.unwrap(); // session, 2 chunks
        for i in 0..3 {
            std::fs::write(dir.join("n.txt"), format!("n{i}")).unwrap();
            d.upload_file("", &format!("docs/n{i}.txt"), &dir.join("n.txt"), 2, &job).await.unwrap();
        }
        state.lock().unwrap().expire_next = true; // survive an expired token
        let mut keys = d.list_all("", "docs/").await.unwrap();
        keys.sort();
        assert_eq!(
            keys,
            [
                ("docs/big.bin".to_string(), size),
                ("docs/n0.txt".into(), 2),
                ("docs/n1.txt".into(), 2),
                ("docs/n2.txt".into(), 2),
                ("docs/sub/a b#1.txt".into(), 14)
            ]
        );
        let files = d.list("", "docs/sub/").await.unwrap();
        assert_eq!(
            files[0].hash.as_deref(),
            Some(format!("sha1:{}", hex(&sha1::Sha1::digest(b"hello onedrive"))).as_str())
        );
        // Same name again: replaced, not duplicated.
        std::fs::write(dir.join("a.txt"), "hello again").unwrap();
        d.upload_file("", "docs/sub/a b#1.txt", &dir.join("a.txt"), 11, &job).await.unwrap();
        assert_eq!(d.list("", "docs/sub/").await.unwrap()[0].size, 11);
        d.download_file("", "docs/big.bin", &dir.join("out.bin"), &job).await.unwrap();
        assert!(std::fs::read(dir.join("out.bin")).unwrap() == big, "download differs");
        assert_eq!(state.lock().unwrap().ranged, 4, "a 55 MiB file comes down as 4 ranges");
        let (part, total) = d.read_range("docs/big.bin", 5, Some(14), 1 << 20).await.unwrap();
        assert_eq!((part.as_slice(), total), (&big[5..15], Some(size)));
        assert!(d.link("", "docs/n0.txt", 7 * 86_400, false).await.unwrap().starts_with("https://1drv.ms/"));
        d.mkdir("", "docs/", "made").await.unwrap();
        assert!(d.mkdir("", "docs/", "made").await.is_err(), "mkdir never merges into an existing folder");
        assert!(d.list("", "docs/").await.unwrap().iter().any(|e| e.key == "docs/made/"));
        // Rename, copy (async, through the monitor URL), and a taken name.
        d.transfer_within("", "docs/n0.txt", "docs/renamed.txt", false, None).await.unwrap();
        assert!(d.list("", "docs/").await.unwrap().iter().any(|e| e.key == "docs/renamed.txt"));
        d.transfer_within("", "docs/renamed.txt", "docs/made/deeper/renamed.txt", true, None).await.unwrap();
        assert!(d.list("", "docs/made/deeper/").await.unwrap().iter().any(|e| e.key == "docs/made/deeper/renamed.txt"));
        assert!(d.transfer_within("", "docs/n1.txt", "docs/renamed.txt", true, None).await.is_err());
        assert_eq!(d.delete("", vec!["docs/".into(), "gone.txt".into()], None).await.unwrap(), 1);
        assert!(state.lock().unwrap().items.is_empty(), "everything went to the (mock) recycle bin");
        assert!(state.lock().unwrap().refreshes >= 2, "the expired token was refreshed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn paths_and_pkce() {
        let d = OneDrive::with(&Profile::default(), String::new(), Arc::new(|_: &str| {}), Endpoints::default());
        assert_eq!(d.item(""), "https://graph.microsoft.com/v1.0/me/drive/root");
        assert_eq!(d.item("a b/c#1.txt"), "https://graph.microsoft.com/v1.0/me/drive/root:/a%20b/c%231.txt:");
        assert_eq!(d.item("Ảnh/"), "https://graph.microsoft.com/v1.0/me/drive/root:/%E1%BA%A2nh:");
        let (v, c) = pkce();
        assert_eq!(v.len(), 64);
        assert_eq!(c, base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())));
        assert!(!c.contains('=') && !c.contains('+') && !c.contains('/'));
    }
}
