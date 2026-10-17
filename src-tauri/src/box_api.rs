//! Box provider (ADR-027). Bring-your-own OAuth app: the user creates a Box "Custom App" (user
//! authentication), pastes its Client ID/secret, and signs in through the browser.
//!
//! - Box addresses everything by ID; NoExplorer by path. A path → ID cache is filled by every
//!   listing, and paths are resolved by listing parents on demand (Box forbids duplicate names in a
//!   folder, so a path is unambiguous).
//! - Tokens: the access token lives in memory (60 min); the refresh token rotates on every refresh
//!   and is saved to the keychain each time (`box-refresh:<profile id>`). Refreshing is serialized,
//!   and a 401 triggers one refresh + retry; 429 waits for Retry-After.
//! - Uploads: < 50 MB as one multipart POST (built by hand, streamed); larger files through an
//!   upload session (parts in parallel, SHA-1 per part and for the whole file, then commit).
//!   An existing file gets a new version instead of a conflict, like overwriting on S3.
//! - Deletes go to the Box trash. Previews stream through `proxy.rs`; "Copy link" creates an
//!   expiring open shared link.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use futures::{StreamExt, TryStreamExt};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::config::Profile;
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry};
use crate::secrets::{OsStore, SecretStore};
use crate::webdav::http;

/// Files at or above this go through an upload session (Box's single-upload limit is 50 MB).
const SESSION_THRESHOLD: u64 = 50 * 1024 * 1024;
const PART_CONCURRENCY: usize = 4;
const WALK_CONCURRENCY: usize = 4;
/// Redirect URI the user registers in their Box app (fixed port, like rclone's).
pub const REDIRECT_URI: &str = "http://127.0.0.1:53682/callback";
const LOOPBACK: &str = "127.0.0.1:53682";

/// Keychain account for a profile's Box refresh token.
pub fn refresh_key(profile_id: &str) -> String {
    format!("box-refresh:{profile_id}")
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Item {
    id: u64,
    dir: bool,
}

struct Tokens {
    access: String,
    expires: Instant,
    refresh: String,
}

/// API endpoints (overridable for the mock server in tests).
#[derive(Clone)]
pub struct Endpoints {
    pub api: String,
    pub upload: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints { api: "https://api.box.com".into(), upload: "https://upload.box.com/api".into() }
    }
}

pub struct BoxCom {
    profile: Profile,
    http: reqwest::Client,
    ep: Endpoints,
    tokens: tokio::sync::Mutex<Tokens>,
    /// Persists a rotated refresh token.
    save_refresh: Arc<dyn Fn(&str) + Send + Sync>,
    /// Path (folders end with `/`, root = "") → Box item.
    ids: Mutex<HashMap<String, Item>>,
}

impl BoxCom {
    /// An account whose refresh token is in the keychain.
    pub fn new(p: &Profile) -> BoxCom {
        let key = refresh_key(&p.id);
        let refresh = OsStore::global().and_then(|s| s.get(&key)).unwrap_or_default();
        let save = move |t: &str| {
            if let Some(s) = OsStore::global() {
                s.set(&key, t);
            }
        };
        BoxCom::with(p, refresh, Arc::new(save), Endpoints::default())
    }

    pub fn with(p: &Profile, refresh: String, save_refresh: Arc<dyn Fn(&str) + Send + Sync>, ep: Endpoints) -> BoxCom {
        let ids = HashMap::from([(String::new(), Item { id: 0, dir: true })]);
        BoxCom {
            profile: p.clone(),
            http: http(),
            ep,
            tokens: tokio::sync::Mutex::new(Tokens { access: String::new(), expires: Instant::now(), refresh }),
            save_refresh,
            ids: Mutex::new(ids),
        }
    }

    pub fn account(&self) -> &str {
        &self.profile.id
    }

    // ---------- auth + requests ----------

    /// A valid access token, refreshing (and saving the rotated refresh token) when needed.
    async fn access(&self, force: bool) -> Result<String, String> {
        let mut t = self.tokens.lock().await;
        if !force && !t.access.is_empty() && Instant::now() < t.expires {
            return Ok(t.access.clone());
        }
        if t.refresh.is_empty() {
            return Err("Not signed in to Box. Open Settings → Accounts and sign in again.".into());
        }
        let r = self
            .http
            .post(format!("{}/oauth2/token", self.ep.api))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &t.refresh),
                ("client_id", self.profile.access_key.trim()),
                ("client_secret", self.profile.secret_key.trim()),
            ]))
            .send()
            .await
            .map_err(net_err)?;
        if r.status() == StatusCode::BAD_REQUEST || r.status() == StatusCode::UNAUTHORIZED {
            return Err("Box sign-in expired. Open Settings → Accounts and sign in again.".into());
        }
        let v: Value = ok(r).await?.json().await.map_err(net_err)?;
        let (Some(access), Some(refresh)) = (v["access_token"].as_str(), v["refresh_token"].as_str()) else {
            return Err("Box didn't return tokens".into());
        };
        let ttl = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(120);
        t.access = access.to_string();
        t.refresh = refresh.to_string();
        t.expires = Instant::now() + Duration::from_secs(ttl);
        (self.save_refresh)(refresh);
        Ok(t.access.clone())
    }

    /// Sends a request built by `build` with auth; refreshes once on 401, waits on 429.
    async fn send(&self, build: impl Fn(&reqwest::Client) -> RequestBuilder) -> Result<Response, String> {
        let mut force = false;
        for attempt in 0..4 {
            let token = self.access(force).await?;
            let r = build(&self.http).bearer_auth(token).send().await.map_err(net_err)?;
            match r.status() {
                StatusCode::UNAUTHORIZED if !force => force = true,
                StatusCode::TOO_MANY_REQUESTS if attempt < 3 => {
                    let wait = r
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(2u64);
                    tokio::time::sleep(Duration::from_secs(wait.min(30))).await;
                }
                _ => return Ok(r),
            }
        }
        Err("Box is busy (too many requests). Try again in a minute.".into())
    }

    async fn json(&self, build: impl Fn(&reqwest::Client) -> RequestBuilder) -> Result<Value, String> {
        ok(self.send(build).await?).await?.json().await.map_err(net_err)
    }

    fn api(&self, path: &str) -> String {
        format!("{}/2.0/{path}", self.ep.api)
    }

    // ---------- paths ----------

    /// Lists a folder (all pages), caching every child's path.
    async fn children(&self, folder_path: &str, id: u64) -> Result<Vec<Entry>, String> {
        let mut out = vec![];
        let mut offset = 0u64;
        loop {
            let url = self.api(&format!(
                "folders/{id}/items?fields=id,type,name,size,modified_at,sha1&limit=1000&offset={offset}"
            ));
            let v = self.json(|h| h.get(&url)).await?;
            let entries = v["entries"].as_array().cloned().unwrap_or_default();
            let mut ids = self.ids.lock().unwrap();
            for e in &entries {
                let (Some(name), Some(item_id)) = (e["name"].as_str(), e["id"].as_str().and_then(|s| s.parse().ok()))
                else {
                    continue;
                };
                let dir = e["type"] == "folder";
                if e["type"] == "web_link" {
                    continue; // bookmarks have no content
                }
                let key = if dir { format!("{folder_path}{name}/") } else { format!("{folder_path}{name}") };
                ids.insert(key.clone(), Item { id: item_id, dir });
                out.push(Entry {
                    key,
                    name: name.to_string(),
                    size: if dir { 0 } else { e["size"].as_u64().unwrap_or(0) },
                    modified: e["modified_at"].as_str().and_then(parse_rfc3339),
                    dir,
                    hash: e["sha1"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("sha1:{}", s.to_ascii_lowercase())),
                });
            }
            drop(ids);
            let total = v["total_count"].as_u64().unwrap_or(0);
            offset += entries.len() as u64;
            if entries.is_empty() || offset >= total {
                return Ok(out);
            }
        }
    }

    /// The Box item at `path` (folders end with `/`), listing parents as needed.
    async fn resolve(&self, path: &str) -> Result<Item, String> {
        if let Some(i) = self.ids.lock().unwrap().get(path) {
            return Ok(*i);
        }
        // Walk down from the deepest cached ancestor.
        let segs: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        let mut parent = String::new();
        for (n, seg) in segs.iter().enumerate() {
            let last = n == segs.len() - 1;
            let here = if last && !path.ends_with('/') { format!("{parent}{seg}") } else { format!("{parent}{seg}/") };
            let known = self.ids.lock().unwrap().get(&here).copied();
            if known.is_none() {
                let p = self.ids.lock().unwrap().get(&parent).copied().ok_or("Not found")?;
                self.children(&parent, p.id).await?;
                if !self.ids.lock().unwrap().contains_key(&here) {
                    return Err(format!("Not found: {}", path.trim_end_matches('/')));
                }
            }
            parent = here;
        }
        self.ids.lock().unwrap().get(path).copied().ok_or_else(|| "Not found".to_string())
    }

    /// Folder `path` (ending with `/`), created with its parents when missing.
    async fn ensure_folder(&self, path: &str) -> Result<u64, String> {
        if let Ok(i) = self.resolve(path).await {
            return Ok(i.id);
        }
        let mut cur = String::new();
        let mut id = 0u64;
        for seg in path.trim_end_matches('/').split('/').filter(|s| !s.is_empty()) {
            let next = format!("{cur}{seg}/");
            id = match self.resolve(&next).await {
                Ok(i) => i.id,
                Err(_) => self.create_folder(id, seg, &next).await?,
            };
            cur = next;
        }
        Ok(id)
    }

    async fn create_folder(&self, parent: u64, name: &str, key: &str) -> Result<u64, String> {
        let url = self.api("folders?fields=id");
        let body = json!({ "name": name, "parent": { "id": parent.to_string() } });
        let r = self.send(|h| h.post(&url).json(&body)).await?;
        let id = if r.status() == StatusCode::CONFLICT {
            // Someone (a parallel upload) made it first: Box names it in the conflict.
            let v: Value = r.json().await.map_err(net_err)?;
            v["context_info"]["conflicts"][0]["id"].as_str().and_then(|s| s.parse().ok()).ok_or("Folder exists")?
        } else {
            let v: Value = ok(r).await?.json().await.map_err(net_err)?;
            v["id"].as_str().and_then(|s| s.parse().ok()).ok_or("Box didn't return the folder id")?
        };
        self.ids.lock().unwrap().insert(key.to_string(), Item { id, dir: true });
        Ok(id)
    }

    // ---------- provider methods ----------

    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        let url = self.api("users/me?fields=id,login");
        self.json(|h| h.get(&url)).await?; // doubles as the connection test
        Ok(vec![BucketInfo { name: "All Files".into(), region: None }])
    }

    pub async fn list(&self, _container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let it = self.resolve(prefix).await?;
        if !it.dir {
            return Err("Not a folder".into());
        }
        self.children(prefix, it.id).await
    }

    pub async fn list_all(&self, container: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        if !prefix.is_empty() && !prefix.ends_with('/') {
            self.resolve(prefix).await?;
            let parent = parent_of(prefix);
            let l = self.list(container, &parent).await?;
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

    /// `inline`: a preview URL through the local proxy; otherwise an open shared link that
    /// expires after `secs` (Box may refuse expiry on free accounts: its message is shown).
    pub async fn link(&self, container: &str, key: &str, secs: u64, inline: bool) -> Result<String, String> {
        if inline {
            return Ok(crate::proxy::url(self.account(), container, key));
        }
        let it = self.resolve(key).await?;
        let kind = if it.dir { "folders" } else { "files" };
        let url = self.api(&format!("{kind}/{}?fields=shared_link", it.id));
        let until = rfc3339_in(secs.clamp(86_400, 365 * 86_400));
        let body = json!({ "shared_link": { "access": "open", "unshared_at": until } });
        let v = self.json(|h| h.put(&url).json(&body)).await?;
        v["shared_link"]["url"].as_str().map(String::from).ok_or_else(|| "Box didn't return a link".into())
    }

    pub async fn read_range(
        &self,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        let it = self.resolve(key).await?;
        let last = end.unwrap_or(start + cap - 1).min(start + cap - 1);
        let url = self.api(&format!("files/{}/content", it.id));
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
        self.create_folder(parent, name, &format!("{prefix}{name}/")).await.map(|_| ())
    }

    /// Moves files and folders (recursively) to the Box trash. Returns how many went.
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
                j.files_done.store(i as u64, Ordering::Relaxed); // the ones before this are done
            }
            let Ok(it) = self.resolve(&k).await else { continue };
            let url = if it.dir {
                self.api(&format!("folders/{}?recursive=true", it.id))
            } else {
                self.api(&format!("files/{}", it.id))
            };
            let r = self.send(|h| h.delete(&url)).await?;
            if r.status() != StatusCode::NOT_FOUND {
                ok(r).await?;
                n += 1;
            }
            self.ids.lock().unwrap().retain(|p, _| !p.starts_with(&k));
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
        let parent_path = parent_of(key);
        let parent = self.ensure_folder(&parent_path).await?;
        let name = key.rsplit('/').next().unwrap_or(key).to_string();
        let existing = self.resolve(key).await.ok().filter(|i| !i.dir).map(|i| i.id);
        let id = if size >= SESSION_THRESHOLD {
            self.upload_session(parent, existing, &name, path, size, job).await?
        } else {
            self.upload_simple(parent, existing, &name, path, size, job).await?
        };
        self.ids.lock().unwrap().insert(key.to_string(), Item { id, dir: false });
        Ok(())
    }

    /// One multipart POST (attributes, then the file), streamed from disk. Returns the file id.
    async fn upload_simple(
        &self,
        parent: u64,
        existing: Option<u64>,
        name: &str,
        path: &Path,
        size: u64,
        job: &Job,
    ) -> Result<u64, String> {
        let url = match existing {
            Some(id) => format!("{}/2.0/files/{id}/content", self.ep.upload),
            None => format!("{}/2.0/files/content", self.ep.upload),
        };
        let attrs = json!({ "name": name, "parent": { "id": parent.to_string() } }).to_string();
        let boundary = format!("noexplorer-{}", uuid::Uuid::new_v4().simple());
        let (head, tail) = multipart_frame(&boundary, &attrs);
        let len = head.len() as u64 + size + tail.len() as u64;
        let mut retried = false;
        let mut url = url;
        loop {
            let (head, tail) = (head.clone(), tail.clone());
            let r = self
                .send(|h| {
                    let body =
                        futures::stream::once(futures::future::ready(Ok::<Vec<u8>, std::io::Error>(head.clone())))
                            .chain(crate::webdav::file_stream(path.to_path_buf(), 0, size, job))
                            .chain(futures::stream::once(futures::future::ready(Ok(tail.clone()))));
                    h.post(&url)
                        .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
                        .header("Content-Length", len)
                        .body(reqwest::Body::wrap_stream(body))
                })
                .await?;
            if r.status() == StatusCode::CONFLICT && !retried {
                // Appeared meanwhile: upload a new version of it instead.
                let v: Value = r.json().await.map_err(net_err)?;
                let id = v["context_info"]["conflicts"]["id"].as_str().ok_or("Name conflict on Box")?;
                url = format!("{}/2.0/files/{id}/content", self.ep.upload);
                retried = true;
                job.bytes_done.fetch_sub(size.min(job.bytes_done.load(Ordering::Relaxed)), Ordering::Relaxed);
                continue;
            }
            let v: Value = ok(r).await?.json().await.map_err(net_err)?;
            return v["entries"][0]["id"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "Box didn't return the file id".into());
        }
    }

    /// Upload session: parts in parallel (SHA-1 each), whole-file SHA-1 at commit. Returns the file id.
    async fn upload_session(
        &self,
        parent: u64,
        existing: Option<u64>,
        name: &str,
        path: &Path,
        size: u64,
        job: &Job,
    ) -> Result<u64, String> {
        let (url, body) = match existing {
            Some(id) => (format!("{}/2.0/files/{id}/upload_sessions", self.ep.upload), json!({ "file_size": size })),
            None => (
                format!("{}/2.0/files/upload_sessions", self.ep.upload),
                json!({ "folder_id": parent.to_string(), "file_size": size, "file_name": name }),
            ),
        };
        let s = self.json(|h| h.post(&url).json(&body)).await?;
        let sid = s["id"].as_str().ok_or("Box didn't open an upload session")?.to_string();
        let part = s["part_size"].as_u64().filter(|p| *p > 0).ok_or("Box didn't give a part size")?;
        let session = format!("{}/2.0/files/upload_sessions/{sid}", self.ep.upload);

        let res: Result<u64, String> = async {
            let whole = Arc::new(Mutex::new(Sha1::new()));
            // Parts are read in order (so the whole-file hash is right) and sent 4 at a time.
            let reader = {
                let (path, whole) = (path.to_path_buf(), whole.clone());
                futures::stream::unfold((None::<tokio::fs::File>, 0u64), move |(f, offset)| {
                    let (path, whole) = (path.clone(), whole.clone());
                    async move {
                        if offset >= size {
                            return None;
                        }
                        let mut f = match f {
                            Some(f) => f,
                            None => match tokio::fs::File::open(&path).await {
                                Ok(f) => f,
                                Err(e) => return Some((Err(e.to_string()), (None, size))),
                            },
                        };
                        let len = part.min(size - offset);
                        let mut buf = vec![0u8; len as usize];
                        if let Err(e) = f.read_exact(&mut buf).await {
                            return Some((Err(e.to_string()), (None, size)));
                        }
                        whole.lock().unwrap().update(&buf);
                        Some((Ok((offset, buf)), (Some(f), offset + len)))
                    }
                })
            };
            let mut parts: Vec<Value> = reader
                .map(|r| {
                    let session = session.clone();
                    async move {
                        let (offset, buf) = r?;
                        if job.cancelled() {
                            return Err("cancelled".to_string());
                        }
                        let digest = b64(&Sha1::digest(&buf));
                        let range = format!("bytes {offset}-{}/{size}", offset + buf.len() as u64 - 1);
                        let n = buf.len() as u64;
                        let v = self
                            .json(|h| {
                                h.put(&session)
                                    .header("Content-Type", "application/octet-stream")
                                    .header("Digest", format!("sha={digest}"))
                                    .header("Content-Range", range.clone())
                                    .body(buf.clone())
                            })
                            .await?;
                        job.bytes_done.fetch_add(n, Ordering::Relaxed);
                        Ok::<Value, String>(v["part"].clone())
                    }
                })
                .buffer_unordered(PART_CONCURRENCY)
                .try_collect()
                .await?;
            parts.sort_by_key(|p| p["offset"].as_u64().unwrap_or(0));
            let digest = b64(&whole.lock().unwrap().clone().finalize());
            let commit = format!("{session}/commit");
            let body = json!({ "parts": parts });
            // 202 = Box is still assembling: retry after the delay it asks for.
            for _ in 0..20 {
                let r = self.send(|h| h.post(&commit).header("Digest", format!("sha={digest}")).json(&body)).await?;
                if r.status() == StatusCode::ACCEPTED {
                    let wait = r
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(2u64);
                    tokio::time::sleep(Duration::from_secs(wait.min(30))).await;
                    continue;
                }
                let v: Value = ok(r).await?.json().await.map_err(net_err)?;
                return v["entries"][0]["id"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| "Box didn't return the file id".into());
            }
            Err("Box took too long to finish the upload".into())
        }
        .await;
        if res.is_err() {
            let _ = self.send(|h| h.delete(&session)).await;
        }
        res
    }

    /// Streams a file to `path` via `path.part`, renaming on success.
    pub async fn download_file(&self, _container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        let it = self.resolve(key).await?;
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".part");
        let tmp = PathBuf::from(tmp);
        let url = self.api(&format!("files/{}/content", it.id));
        let r = ok(self.send(|h| h.get(&url)).await?).await?;
        let mut body = r.bytes_stream();
        let f = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
        let mut w = tokio::io::BufWriter::with_capacity(1 << 20, f);
        let res: Result<(), String> = async {
            while let Some(chunk) = body.try_next().await.map_err(net_err)? {
                if job.cancelled() {
                    return Err("cancelled".into());
                }
                w.write_all(&chunk).await.map_err(|e| e.to_string())?;
                job.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
            w.flush().await.map_err(|e| e.to_string())
        }
        .await;
        drop(w);
        match res {
            Ok(()) => tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string()),
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                Err(e)
            }
        }
    }
}

// ---------- sign-in ----------

/// Signs in to Box with the user's own OAuth app (Client ID + secret) through the browser.
///
/// Listens on [`REDIRECT_URI`] for the callback, exchanges the code, saves the refresh token in the
/// keychain and the account (with its client secret, also in the keychain). Returns the settings.
#[tauri::command]
pub async fn box_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
    client_id: String,
    client_secret: String,
    id: Option<String>,
    name: String,
) -> Result<crate::config::Settings, String> {
    use tauri_plugin_opener::OpenerExt;
    let s = state.settings();
    let existing = id.as_deref().and_then(|id| s.profiles.iter().find(|p| p.id == id).cloned());
    // An empty secret on re-sign-in means "keep the saved one".
    let client_secret = match client_secret.trim() {
        "" => existing.as_ref().map(|p| p.secret_key.clone()).unwrap_or_default(),
        s => s.to_string(),
    };
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err("Enter your Box app's Client ID and Client secret first".into());
    }
    let listener = tokio::net::TcpListener::bind(LOOPBACK)
        .await
        .map_err(|e| format!("Can't listen on {LOOPBACK} for the Box sign-in ({e}). Is another sign-in open?"))?;
    crate::webdav::LOGIN_CANCEL.store(false, Ordering::Relaxed);
    let csrf = uuid::Uuid::new_v4().simple().to_string();
    let auth = format!(
        "https://account.box.com/api/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={csrf}",
        enc(&client_id),
        enc(REDIRECT_URI)
    );
    app.opener().open_url(&auth, None::<&str>).map_err(|e| e.to_string())?;
    let code = wait_for_code(&listener, &csrf).await?;
    drop(listener);

    let r = http()
        .post("https://api.box.com/oauth2/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("redirect_uri", REDIRECT_URI),
        ]))
        .send()
        .await
        .map_err(net_err)?;
    let v: Value = ok(r).await?.json().await.map_err(net_err)?;
    let refresh = v["refresh_token"].as_str().ok_or("Box didn't return a refresh token")?.to_string();

    let mut p = existing.unwrap_or_default();
    if p.id.is_empty() {
        p.id = uuid::Uuid::new_v4().to_string();
    }
    p.kind = crate::config::ProviderKind::Box;
    p.access_key = client_id;
    p.secret_key = client_secret;
    p.endpoint.clear();
    // Name it after the Box login unless the user typed a name.
    let probe = BoxCom::with(&p, refresh.clone(), Arc::new(|_: &str| {}), Endpoints::default());
    let me = probe.json(|h| h.get("https://api.box.com/2.0/users/me?fields=login")).await?;
    let rotated = probe.tokens.lock().await.refresh.clone(); // the probe refreshed once
    p.name = match name.trim() {
        "" => me["login"].as_str().map(|l| format!("Box · {l}")).unwrap_or_else(|| "Box".into()),
        n => n.to_string(),
    };
    let store = OsStore::new(&app);
    if !store.set(&refresh_key(&p.id), &rotated) {
        return Err("Couldn't save the Box sign-in in the system keychain".into());
    }
    state.forget(&p.id);
    crate::config::store_account(&app, &state, p)
}

/// Accepts connections on the loopback listener until Box redirects back with `code`.
async fn wait_for_code(listener: &tokio::net::TcpListener, csrf: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    loop {
        if crate::webdav::LOGIN_CANCEL.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("Sign-in timed out. Try again.".into());
        }
        let Ok(Ok((mut sock, _))) = tokio::time::timeout(Duration::from_secs(1), listener.accept()).await else {
            continue;
        };
        let mut buf = vec![0u8; 8192];
        let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf))
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
        let Some(query) = target.strip_prefix("/callback?") else {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        };
        let params = parse_query(query);
        let (msg, result) = if params.get("state").map(String::as_str) != Some(csrf) {
            (
                "This sign-in link doesn't match. Start again from NoExplorer.",
                Err("Sign-in didn't match (state)".to_string()),
            )
        } else if let Some(code) = params.get("code") {
            ("NoExplorer is connected to Box. You can close this tab.", Ok(code.clone()))
        } else {
            let why = params.get("error_description").or(params.get("error")).cloned().unwrap_or("denied".into());
            ("Box sign-in was not completed. You can close this tab.", Err(format!("Box sign-in failed: {why}")))
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>NoExplorer</title><body style=\"font:16px system-ui;padding:3em;text-align:center\"><p>{msg}</p></body>"
        );
        let _ = sock
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len())
                    .as_bytes(),
            )
            .await;
        return result;
    }
}

// ---------- helpers ----------

fn parent_of(key: &str) -> String {
    match key.trim_end_matches('/').rfind('/') {
        Some(i) => key[..=i].to_string(),
        None => String::new(),
    }
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            let d =
                |s: &str| percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
            (d(k), d(v))
        })
        .collect()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Multipart framing around the file bytes: `attributes` JSON first (Box requires it before the file).
fn multipart_frame(boundary: &str, attrs: &str) -> (Vec<u8>, Vec<u8>) {
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"attributes\"\r\n\r\n{attrs}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"file\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    );
    (head.into_bytes(), format!("\r\n--{boundary}--\r\n").into_bytes())
}

fn net_err(e: reqwest::Error) -> String {
    if e.is_connect() {
        "Can't reach Box (check your connection)".into()
    } else if e.is_timeout() {
        "Box took too long to answer".into()
    } else {
        e.to_string()
    }
}

/// Passes 2xx through; turns Box's JSON errors into readable messages.
async fn ok(r: Response) -> Result<Response, String> {
    let st = r.status();
    if st.is_success() {
        return Ok(r);
    }
    let v: Value = r.json().await.unwrap_or(Value::Null);
    let msg = v["message"].as_str().or(v["error_description"].as_str()).unwrap_or("").to_string();
    let what = match st.as_u16() {
        401 => "Box sign-in expired. Sign in again in Settings → Accounts",
        403 => "Not allowed on Box",
        404 => "Not found on Box",
        409 => "Something with that name already exists",
        413 => "File too large for this Box account",
        507 => "Not enough space on Box",
        _ => "Box request failed",
    };
    Err(if msg.is_empty() {
        format!("{what} (HTTP {})", st.as_u16())
    } else {
        format!("{what} (HTTP {}): {msg}", st.as_u16())
    })
}

/// `2026-10-16T10:01:02-07:00` → Unix millis.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()?? as u32, d.next()?? as u32);
    let time = &rest[..8.min(rest.len())];
    let hms: Vec<i64> = time.split(':').filter_map(|x| x.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    let tz = &rest[8..];
    let tz = tz.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match tz {
        "" | "Z" => 0,
        t => {
            let sign = if t.starts_with('-') { -1 } else { 1 };
            let hm: Vec<i64> = t[1..].split(':').filter_map(|x| x.parse().ok()).collect();
            sign * (hm.first().copied().unwrap_or(0) * 3600 + hm.get(1).copied().unwrap_or(0) * 60)
        }
    };
    let days = crate::webdav::days_from_civil(y, m, day);
    Some((days * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2] - offset) * 1000)
}

/// RFC 3339 timestamp `secs` from now (UTC).
fn rfc3339_in(secs: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() + secs;
    let (y, m, d) = crate::webdav::civil_from_days((now / 86_400) as i64);
    let t = now % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- a tiny in-memory Box API, enough to drive BoxCom end to end ----------

    #[derive(Clone)]
    struct Node {
        parent: u64,
        name: String,
        dir: bool,
        data: Vec<u8>,
    }

    /// Upload session: (parent folder, name, size, existing file, parts by offset).
    type Session = (u64, String, u64, Option<u64>, Vec<(u64, Vec<u8>)>);

    #[derive(Default)]
    struct Mock {
        next: u64,
        nodes: HashMap<u64, Node>,
        sessions: HashMap<String, Session>,
        refresh: String,
        access: String,
        /// Answer the next API call with 401 (to test refresh + retry).
        expire_next: bool,
        refreshes: u32,
    }

    impl Mock {
        fn id(&mut self) -> u64 {
            self.next += 1;
            self.next
        }
        fn child(&self, parent: u64, name: &str) -> Option<u64> {
            self.nodes.iter().find(|(_, n)| n.parent == parent && n.name == name).map(|(i, _)| *i)
        }
        fn file_json(&self, id: u64) -> Value {
            let n = &self.nodes[&id];
            json!({ "type": if n.dir { "folder" } else { "file" }, "id": id.to_string(), "name": n.name,
                    "size": n.data.len(), "modified_at": "2026-10-16T10:00:00-07:00",
                    "sha1": if n.dir { String::new() } else { hex(&Sha1::digest(&n.data)) } })
        }
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Reads one HTTP request (Content-Length or chunked body).
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
        if let Some(len) = headers.get("content-length").and_then(|l| l.parse::<usize>().ok()) {
            while body.len() < len {
                let n = sock.read(&mut tmp).await.ok()?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..n]);
            }
        } else if headers.get("transfer-encoding").is_some_and(|t| t.contains("chunked")) {
            loop {
                if let Some(end) = find_chunked_end(&body) {
                    body = dechunk(&body[..end]);
                    break;
                }
                let n = sock.read(&mut tmp).await.ok()?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..n]);
            }
        }
        Some((method, target, headers, body))
    }

    fn find_chunked_end(b: &[u8]) -> Option<usize> {
        b.windows(5).position(|w| w == b"0\r\n\r\n").map(|i| i + 5)
    }

    fn dechunk(mut b: &[u8]) -> Vec<u8> {
        let mut out = vec![];
        loop {
            let Some(i) = b.windows(2).position(|w| w == b"\r\n") else { return out };
            let len = usize::from_str_radix(String::from_utf8_lossy(&b[..i]).trim(), 16).unwrap_or(0);
            if len == 0 {
                return out;
            }
            out.extend_from_slice(&b[i + 2..i + 2 + len]);
            b = &b[i + 2 + len + 2..];
        }
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

    /// Multipart body → (attributes JSON, file bytes).
    fn parse_multipart(body: &[u8], boundary: &str) -> (Value, Vec<u8>) {
        let text = |b: &[u8]| String::from_utf8_lossy(b).to_string();
        let sep = format!("--{boundary}");
        let pos = |hay: &[u8], needle: &[u8], from: usize| {
            hay[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
        };
        let a0 = pos(body, b"\r\n\r\n", 0).unwrap() + 4;
        let a1 = pos(body, format!("\r\n{sep}").as_bytes(), a0).unwrap();
        let attrs: Value = serde_json::from_str(&text(&body[a0..a1])).unwrap();
        let f0 = pos(body, b"\r\n\r\n", a1 + 2).unwrap() + 4;
        let f1 = pos(body, format!("\r\n{sep}--").as_bytes(), f0).unwrap();
        (attrs, body[f0..f1].to_vec())
    }

    async fn serve(state: Arc<Mutex<Mock>>, mut sock: tokio::net::TcpStream) {
        let Some((method, target, headers, body)) = read_req(&mut sock).await else { return };
        let (path, query) = target.split_once('?').unwrap_or((&target, ""));
        let path = path.to_string();
        let reply = {
            let mut m = state.lock().unwrap();
            if path == "/oauth2/token" {
                let f = parse_query(&String::from_utf8_lossy(&body));
                if f.get("refresh_token") != Some(&m.refresh) {
                    respond(400, br#"{"error":"invalid_grant"}"#, "")
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
            } else if headers.get("authorization") != Some(&format!("Bearer {}", m.access)) || m.expire_next {
                m.expire_next = false;
                respond(401, br#"{"message":"expired"}"#, "")
            } else {
                let seg: Vec<&str> = path.trim_start_matches('/').split('/').collect();
                match (method.as_str(), seg.as_slice()) {
                    ("GET", ["2.0", "users", "me"]) => respond(200, br#"{"id":"1","login":"dev@example.com"}"#, ""),
                    ("GET", ["2.0", "folders", id, "items"]) => {
                        let id: u64 = id.parse().unwrap();
                        let mut kids: Vec<u64> =
                            m.nodes.iter().filter(|(_, n)| n.parent == id).map(|(i, _)| *i).collect();
                        kids.sort();
                        let q = parse_query(query);
                        let (off, lim) = (q["offset"].parse::<usize>().unwrap(), 2usize); // tiny pages: test paging
                        let page: Vec<Value> = kids.iter().skip(off).take(lim).map(|k| m.file_json(*k)).collect();
                        respond(200, json!({ "total_count": kids.len(), "entries": page }).to_string().as_bytes(), "")
                    }
                    ("POST", ["2.0", "folders"]) => {
                        let v: Value = serde_json::from_slice(&body).unwrap();
                        let parent: u64 = v["parent"]["id"].as_str().unwrap().parse().unwrap();
                        let name = v["name"].as_str().unwrap().to_string();
                        if let Some(c) = m.child(parent, &name) {
                            respond(
                                409,
                                json!({ "context_info": { "conflicts": [{ "id": c.to_string() }] } })
                                    .to_string()
                                    .as_bytes(),
                                "",
                            )
                        } else {
                            let id = m.id();
                            m.nodes.insert(id, Node { parent, name, dir: true, data: vec![] });
                            respond(201, json!({ "id": id.to_string() }).to_string().as_bytes(), "")
                        }
                    }
                    ("POST", ["api", "2.0", "files", rest @ ..]) if rest.last() == Some(&"content") => {
                        let boundary = headers["content-type"].split("boundary=").nth(1).unwrap().to_string();
                        let (attrs, data) = parse_multipart(&body, &boundary);
                        let parent: u64 = attrs["parent"]["id"].as_str().unwrap().parse().unwrap();
                        let name = attrs["name"].as_str().unwrap().to_string();
                        let target = if rest.len() == 2 { rest[0].parse::<u64>().ok() } else { None };
                        match (target, m.child(parent, &name)) {
                            (None, Some(c)) => respond(
                                409,
                                json!({ "context_info": { "conflicts": { "id": c.to_string() } } })
                                    .to_string()
                                    .as_bytes(),
                                "",
                            ),
                            (t, c) => {
                                let id = t.or(c).unwrap_or_else(|| m.id());
                                m.nodes.insert(id, Node { parent, name, dir: false, data });
                                respond(201, json!({ "entries": [m.file_json(id)] }).to_string().as_bytes(), "")
                            }
                        }
                    }
                    ("POST", ["api", "2.0", "files", rest @ ..]) if rest.last() == Some(&"upload_sessions") => {
                        let v: Value = serde_json::from_slice(&body).unwrap();
                        let size = v["file_size"].as_u64().unwrap();
                        let (parent, name, existing) = if rest.len() == 2 {
                            let id: u64 = rest[0].parse().unwrap();
                            (m.nodes[&id].parent, m.nodes[&id].name.clone(), Some(id))
                        } else {
                            (
                                v["folder_id"].as_str().unwrap().parse().unwrap(),
                                v["file_name"].as_str().unwrap().to_string(),
                                None,
                            )
                        };
                        let sid = format!("s{}", m.id());
                        m.sessions.insert(sid.clone(), (parent, name, size, existing, vec![]));
                        let part = 8 * 1024 * 1024u64;
                        respond(
                            201,
                            json!({ "id": sid, "part_size": part, "total_parts": size.div_ceil(part) })
                                .to_string()
                                .as_bytes(),
                            "",
                        )
                    }
                    ("PUT", ["api", "2.0", "files", "upload_sessions", sid]) => {
                        let digest = headers["digest"].trim_start_matches("sha=").to_string();
                        assert_eq!(digest, b64(&Sha1::digest(&body)), "part digest");
                        let off: u64 = headers["content-range"].split(['-', ' ']).nth(1).unwrap().parse().unwrap();
                        m.sessions.get_mut(*sid).unwrap().4.push((off, body.clone()));
                        respond(
                            200,
                            json!({ "part": { "part_id": format!("p{off}"), "offset": off, "size": body.len() } })
                                .to_string()
                                .as_bytes(),
                            "",
                        )
                    }
                    ("POST", ["api", "2.0", "files", "upload_sessions", sid, "commit"]) => {
                        let (parent, name, size, existing, mut parts) = m.sessions.remove(*sid).unwrap();
                        parts.sort_by_key(|p| p.0);
                        let data: Vec<u8> = parts.into_iter().flat_map(|p| p.1).collect();
                        assert_eq!(data.len() as u64, size);
                        assert_eq!(
                            headers["digest"].trim_start_matches("sha="),
                            b64(&Sha1::digest(&data)),
                            "whole-file digest"
                        );
                        let id = existing.unwrap_or_else(|| m.id());
                        m.nodes.insert(id, Node { parent, name, dir: false, data });
                        respond(201, json!({ "entries": [m.file_json(id)] }).to_string().as_bytes(), "")
                    }
                    ("GET", ["2.0", "files", id, "content"]) => {
                        let data = m.nodes[&id.parse::<u64>().unwrap()].data.clone();
                        match headers
                            .get("range")
                            .and_then(|r| r.strip_prefix("bytes="))
                            .and_then(|r| r.split_once('-'))
                        {
                            Some((a, b)) => {
                                let a: usize = a.parse().unwrap();
                                let b = b.parse::<usize>().unwrap_or(data.len() - 1).min(data.len() - 1);
                                respond(206, &data[a..=b], &format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()))
                            }
                            None => respond(200, &data, ""),
                        }
                    }
                    ("DELETE", ["2.0", kind, id]) => {
                        let id: u64 = id.parse().unwrap();
                        let mut gone = vec![id];
                        while let Some(g) = gone.pop() {
                            m.nodes.remove(&g);
                            gone.extend(m.nodes.iter().filter(|(_, n)| n.parent == g).map(|(i, _)| *i));
                        }
                        let _ = kind;
                        respond(204, b"", "")
                    }
                    ("PUT", ["2.0", _kind, id]) => respond(
                        200,
                        json!({ "shared_link": { "url": format!("https://app.box.com/s/{id}") } })
                            .to_string()
                            .as_bytes(),
                        "",
                    ),
                    _ => respond(404, br#"{"message":"no route"}"#, ""),
                }
            }
        };
        let _ = sock.write_all(&reply).await;
    }

    #[tokio::test]
    async fn box_roundtrip_against_mock() {
        let state = Arc::new(Mutex::new(Mock { refresh: "refresh-0".into(), ..Default::default() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let st = state.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                tokio::spawn(serve(st.clone(), sock));
            }
        });
        let saved = Arc::new(Mutex::new(String::new()));
        let saved2 = saved.clone();
        let p =
            Profile { id: "bx".into(), access_key: "cid".into(), secret_key: "csecret".into(), ..Default::default() };
        let ep = Endpoints { api: format!("http://{addr}"), upload: format!("http://{addr}/api") };
        let b =
            BoxCom::with(&p, "refresh-0".into(), Arc::new(move |t: &str| *saved2.lock().unwrap() = t.to_string()), ep);

        assert_eq!(b.containers().await.unwrap()[0].name, "All Files");
        assert_eq!(*saved.lock().unwrap(), "refresh-1", "rotated refresh token saved");

        let dir = std::env::temp_dir().join(format!("nx-box-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..(60 * 1024 * 1024 + 9)).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(dir.join("big.bin"), &big).unwrap();
        std::fs::write(dir.join("a.txt"), "hello box").unwrap();
        let job = Job::untracked("up", "t".into(), "bx", "All Files", "");
        b.upload_file("", "docs/sub/a.txt", &dir.join("a.txt"), 9, &job).await.unwrap(); // creates docs/ and sub/
        b.upload_file("", "docs/big.bin", &dir.join("big.bin"), big.len() as u64, &job).await.unwrap(); // session
        for i in 0..3 {
            std::fs::write(dir.join("n.txt"), format!("n{i}")).unwrap();
            b.upload_file("", &format!("docs/n{i}.txt"), &dir.join("n.txt"), 2, &job).await.unwrap();
        }
        // A fresh client (empty path cache) must find everything by listing, across pages.
        state.lock().unwrap().expire_next = true; // and survive an expired token
        let b2 = BoxCom::with(&p, saved.lock().unwrap().clone(), Arc::new(|_: &str| {}), b.ep.clone());
        let mut keys: Vec<(String, u64)> = b2.list_all("", "docs/").await.unwrap();
        keys.sort();
        assert_eq!(
            keys,
            [
                ("docs/big.bin".to_string(), big.len() as u64),
                ("docs/n0.txt".into(), 2),
                ("docs/n1.txt".into(), 2),
                ("docs/n2.txt".into(), 2),
                ("docs/sub/a.txt".into(), 9)
            ]
        );
        let files = b2.list("", "docs/sub/").await.unwrap();
        assert_eq!(files[0].hash.as_deref(), Some(format!("sha1:{}", hex(&Sha1::digest(b"hello box"))).as_str()));
        // A second upload of the same name is a new version, not a duplicate.
        std::fs::write(dir.join("a.txt"), "hello again").unwrap();
        b2.upload_file("", "docs/sub/a.txt", &dir.join("a.txt"), 11, &job).await.unwrap();
        assert_eq!(b2.list("", "docs/sub/").await.unwrap().len(), 1);
        b2.download_file("", "docs/big.bin", &dir.join("out.bin"), &job).await.unwrap();
        assert!(std::fs::read(dir.join("out.bin")).unwrap() == big, "download differs");
        let (part, total) = b2.read_range("docs/big.bin", 5, Some(14), 1 << 20).await.unwrap();
        assert_eq!((part.as_slice(), total), (&big[5..15], Some(big.len() as u64)));
        assert!(b2.link("", "docs/n0.txt", 7 * 86_400, false).await.unwrap().contains("/s/"));
        b2.mkdir("", "docs/", "made").await.unwrap();
        assert!(b2.list("", "docs/").await.unwrap().iter().any(|e| e.key == "docs/made/"));
        assert_eq!(b2.delete("", vec!["docs/".into()], None).await.unwrap(), 1);
        assert!(state.lock().unwrap().nodes.is_empty(), "everything went to the (mock) trash");
        assert!(state.lock().unwrap().refreshes >= 3, "the expired token was refreshed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn helpers() {
        assert_eq!(parent_of("a/b/c.txt"), "a/b/");
        assert_eq!(parent_of("a/b/"), "a/");
        assert_eq!(parent_of("top.txt"), "");
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_rfc3339("1970-01-01T07:00:01+07:00"), Some(1000));
        assert_eq!(parse_rfc3339("2026-10-16T10:01:02-07:00"), parse_rfc3339("2026-10-16T17:01:02Z"));
        let q = parse_query("code=abc%2F1&state=xyz&error_description=a+b");
        assert_eq!((q["code"].as_str(), q["state"].as_str(), q["error_description"].as_str()), ("abc/1", "xyz", "a b"));
        let (h, t) = multipart_frame("B", "{}");
        assert!(
            String::from_utf8(h)
                .unwrap()
                .starts_with("--B\r\nContent-Disposition: form-data; name=\"attributes\"\r\n\r\n{}\r\n--B\r\n")
        );
        assert_eq!(t, b"\r\n--B--\r\n");
        assert_eq!(b64(&Sha1::digest(b"abc")), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
        assert!(rfc3339_in(86_400).ends_with('Z'));
    }
}
