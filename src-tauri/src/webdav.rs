//! WebDAV provider: Nextcloud (and ownCloud), NAS WebDAV and any WebDAV server (ADR-026).
//!
//! - Our own small client on `reqwest` (already in the tree via the updater): PROPFIND (Depth 1),
//!   GET with Range, PUT, MKCOL, DELETE, MOVE. Basic auth with the account's user + (app) password.
//! - Keys are `/`-separated paths below the account's root collection; folders end with `/`.
//!   There is one container ("Files"); its name is only a label.
//! - Nextcloud extras: the root is `<server>/remote.php/dav/files/<user>/`, big files use chunked
//!   upload v2 (parallel-safe, resumable on the server side), and "Copy link" creates a public
//!   share that expires (OCS API). Sign-in uses Login Flow v2 (`nextcloud_login`): the browser
//!   approves, and the app password goes straight to the keychain without passing through the UI.
//! - Previews can't send auth headers, so `link(inline)` returns an `nxr://` URL served by
//!   `proxy.rs`, which streams through this client (with Range support for video).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::{StreamExt, TryStreamExt};
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::config::{Profile, ProviderKind, normalize_endpoint};
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry};
use crate::util::file_name;

/// Nextcloud: files above this go through chunked upload v2, in chunks of this size.
const CHUNK: u64 = 16 * 1024 * 1024;
/// Recursive listings fetch this many folders at once.
const WALK_CONCURRENCY: usize = 6;

/// Characters escaped in one path segment of a URL.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// A shared HTTP client (rustls with the ring provider, like the updater).
pub fn http() -> reqwest::Client {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .user_agent(concat!("NoExplorer/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("HTTP client")
}

fn method(m: &str) -> Method {
    Method::from_bytes(m.as_bytes()).expect("HTTP method")
}

pub(crate) fn enc(seg: &str) -> String {
    utf8_percent_encode(seg, SEGMENT).to_string()
}

/// `a b/c` → `a%20b/c` (keeps the slashes, including a trailing one).
pub(crate) fn enc_path(key: &str) -> String {
    key.split('/').map(enc).collect::<Vec<_>>().join("/")
}

/// Nextcloud: `https://cloud.example.com/nc` (no trailing slash). Accepts a pasted WebDAV URL too.
fn server_of(endpoint: &str) -> String {
    let e = normalize_endpoint(endpoint);
    match e.find("/remote.php/") {
        Some(i) => e[..i].to_string(),
        None => e,
    }
}

/// One WebDAV account.
pub struct Dav {
    profile: Profile,
    http: reqwest::Client,
    /// Root collection URL, ending with `/`.
    root: String,
    /// The root URL's path, decoded (to turn `href`s back into keys).
    root_path: String,
    nextcloud: bool,
    /// Collections known to exist (so uploads don't MKCOL the same parents again).
    made: Mutex<HashSet<String>>,
}

impl Dav {
    pub fn new(p: &Profile) -> Dav {
        let nextcloud = p.kind == ProviderKind::Nextcloud;
        let root = if nextcloud {
            format!("{}/remote.php/dav/files/{}/", server_of(&p.endpoint), enc(p.access_key.trim()))
        } else {
            format!("{}/", normalize_endpoint(&p.endpoint))
        };
        let root_path = decode(url_path(&root));
        Dav { profile: p.clone(), http: http(), root, root_path, nextcloud, made: Mutex::new(HashSet::new()) }
    }

    pub fn account(&self) -> &str {
        &self.profile.id
    }

    fn url(&self, key: &str) -> String {
        format!("{}{}", self.root, enc_path(key))
    }

    fn req(&self, m: Method, url: &str) -> RequestBuilder {
        self.http.request(m, url).basic_auth(self.profile.access_key.trim(), Some(self.profile.secret_key.trim()))
    }

    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        // Doubles as the connection test: the root must be a readable collection.
        self.propfind("", "0").await?;
        Ok(vec![BucketInfo { name: "Files".into(), region: None }])
    }

    /// PROPFIND `key` with `depth`, returning entries (the folder itself included, key as-is).
    async fn propfind(&self, key: &str, depth: &str) -> Result<Vec<Entry>, String> {
        let body = if self.nextcloud {
            r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><oc:checksums/></d:prop></d:propfind>"#
        } else {
            r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#
        };
        let r = self
            .req(method("PROPFIND"), &self.url(key))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body)
            .send()
            .await
            .map_err(net_err)?;
        let xml = ok(r).await?.text().await.map_err(net_err)?;
        Ok(parse_multistatus(&xml)
            .into_iter()
            .filter_map(|p| {
                let path = decode(url_path(&p.href));
                let rel = path.strip_prefix(&self.root_path)?.trim_start_matches('/');
                let key =
                    if p.dir && !rel.is_empty() && !rel.ends_with('/') { format!("{rel}/") } else { rel.to_string() };
                let hash = checksum(&p.checksums);
                Some(Entry { name: file_name(&key), key, size: p.size, modified: p.modified, dir: p.dir, hash })
            })
            .collect())
    }

    pub async fn list(&self, _container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let me = prefix.trim_start_matches('/');
        Ok(self
            .propfind(me, "1")
            .await?
            .into_iter()
            .filter(|e| !e.key.is_empty() && e.key != me)
            .map(|mut e| {
                if e.dir {
                    e.size = 0;
                }
                e
            })
            .collect())
    }

    /// Every file below `prefix`, walking folders level by level (Depth: infinity is often off).
    pub async fn list_all(&self, container: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        if !prefix.is_empty() && !prefix.ends_with('/') {
            // A single file.
            let e = self.propfind(prefix, "0").await?;
            return Ok(e.into_iter().map(|e| (e.key, e.size)).collect());
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

    /// `inline`: a preview URL through the local proxy. Otherwise a public share link (Nextcloud).
    pub async fn link(&self, container: &str, key: &str, secs: u64, inline: bool) -> Result<String, String> {
        if inline {
            return Ok(crate::proxy::url(self.account(), container, key));
        }
        if !self.nextcloud {
            return Err("This server can't create share links (plain WebDAV).".into());
        }
        let days = secs.div_ceil(86_400).clamp(1, 365);
        let expire = date_in_days(days);
        let api =
            format!("{}/ocs/v2.php/apps/files_sharing/api/v1/shares?format=json", server_of(&self.profile.endpoint));
        let r = self
            .req(Method::POST, &api)
            .header("OCS-APIRequest", "true")
            .form_body(&[
                ("path", &format!("/{}", key.trim_end_matches('/'))),
                ("shareType", "3"),
                ("permissions", "1"),
                ("expireDate", &expire),
            ])
            .send()
            .await
            .map_err(net_err)?;
        let v: serde_json::Value = ok(r).await?.json().await.map_err(net_err)?;
        v["ocs"]["data"]["url"].as_str().map(String::from).ok_or_else(|| {
            v["ocs"]["meta"]["message"].as_str().unwrap_or("The server didn't return a link").to_string()
        })
    }

    /// Bytes `start..=end` (or to the end, at most `cap` bytes), plus the file's total size.
    pub async fn read_range(
        &self,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        let last = end.unwrap_or(start + cap - 1).min(start + cap - 1);
        let r = self
            .req(Method::GET, &self.url(key))
            .header("Range", format!("bytes={start}-{last}"))
            .send()
            .await
            .map_err(net_err)?;
        let r = ok(r).await?;
        let total = r
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|n| n.parse().ok())
            .or_else(|| (r.status() == StatusCode::OK).then(|| r.content_length()).flatten());
        let mut bytes = r.bytes().await.map_err(net_err)?.to_vec();
        bytes.truncate(cap as usize); // a server that ignored Range sent everything
        Ok((bytes, total))
    }

    pub async fn read_head(&self, _container: &str, key: &str, max: u64) -> Result<Vec<u8>, String> {
        Ok(self.read_range(key, 0, None, max.max(1)).await?.0)
    }

    pub async fn mkdir(&self, _container: &str, prefix: &str, name: &str) -> Result<(), String> {
        let r = self.req(method("MKCOL"), &self.url(&format!("{prefix}{name}/"))).send().await.map_err(net_err)?;
        ok(r).await.map(|_| ())
    }

    /// Makes sure every folder above `key` exists (MKCOL from the top; "already exists" is fine).
    async fn ensure_parents(&self, key: &str) -> Result<(), String> {
        let mut path = String::new();
        let parts: Vec<&str> = key.split('/').collect();
        for seg in &parts[..parts.len().saturating_sub(1)] {
            path.push_str(seg);
            path.push('/');
            if self.made.lock().unwrap().contains(&path) {
                continue;
            }
            let r = self.req(method("MKCOL"), &self.url(&path)).send().await.map_err(net_err)?;
            // 405 = it already exists.
            if !(r.status().is_success() || r.status() == StatusCode::METHOD_NOT_ALLOWED) {
                return ok(r).await.map(|_| ());
            }
            self.made.lock().unwrap().insert(path.clone());
        }
        Ok(())
    }

    /// Deletes files and folders (WebDAV deletes folders recursively). Returns how many were removed.
    pub async fn delete(&self, _container: &str, keys: Vec<String>) -> Result<usize, String> {
        let mut n = 0;
        for k in keys {
            let r = self.req(Method::DELETE, &self.url(&k)).send().await.map_err(net_err)?;
            if r.status() == StatusCode::NOT_FOUND {
                continue;
            }
            ok(r).await?;
            n += 1;
            self.made.lock().unwrap().retain(|m| !m.starts_with(&k));
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
        self.ensure_parents(key).await?;
        if self.nextcloud && size > CHUNK {
            return self.upload_chunked(key, path, size, job).await;
        }
        let r = self
            .req(Method::PUT, &self.url(key))
            .header("Content-Length", size)
            .body(reqwest::Body::wrap_stream(file_stream(path.to_path_buf(), 0, size, job)))
            .send()
            .await
            .map_err(net_err)?;
        ok(r).await.map(|_| ())
    }

    /// Nextcloud chunked upload v2: MKCOL a session, PUT numbered chunks, MOVE `.file` into place.
    async fn upload_chunked(&self, key: &str, path: &Path, size: u64, job: &Job) -> Result<(), String> {
        let server = server_of(&self.profile.endpoint);
        let user = enc(self.profile.access_key.trim());
        let session = format!("{server}/remote.php/dav/uploads/{user}/noexplorer-{}", uuid::Uuid::new_v4());
        let dest = self.url(key);
        let r = self.req(method("MKCOL"), &session).header("Destination", &dest).send().await.map_err(net_err)?;
        ok(r).await?;
        let res: Result<(), String> = async {
            let chunks = size.div_ceil(CHUNK);
            for i in 0..chunks {
                if job.cancelled() {
                    return Err("cancelled".into());
                }
                let offset = i * CHUNK;
                let len = CHUNK.min(size - offset);
                let r = self
                    .req(Method::PUT, &format!("{session}/{:05}", i + 1))
                    .header("Destination", &dest)
                    .header("OC-Total-Length", size)
                    .header("Content-Length", len)
                    .body(reqwest::Body::wrap_stream(file_stream(path.to_path_buf(), offset, len, job)))
                    .send()
                    .await
                    .map_err(net_err)?;
                ok(r).await?;
            }
            let r = self
                .req(method("MOVE"), &format!("{session}/.file"))
                .header("Destination", &dest)
                .header("OC-Total-Length", size)
                .header("Overwrite", "T")
                .timeout(Duration::from_secs(600)) // assembling a huge file takes a while
                .send()
                .await
                .map_err(net_err)?;
            ok(r).await.map(|_| ())
        }
        .await;
        if res.is_err() {
            let _ = self.req(Method::DELETE, &session).send().await;
        }
        res
    }

    /// Streams a file to `path` via `path.part`, renaming on success.
    pub async fn download_file(&self, _container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".part");
        let tmp = PathBuf::from(tmp);
        let r = ok(self.req(Method::GET, &self.url(key)).send().await.map_err(net_err)?).await?;
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

/// `len` bytes of a file from `offset`, in 1 MB pieces, counting progress on `job`.
pub(crate) fn file_stream(
    path: PathBuf,
    offset: u64,
    len: u64,
    job: &Job,
) -> impl futures::Stream<Item = Result<Vec<u8>, std::io::Error>> + Send + 'static {
    let (done, cancel) = job.progress_handles();
    futures::stream::unfold((None::<tokio::fs::File>, len), move |(f, left)| {
        let (path, done, cancel) = (path.clone(), done.clone(), cancel.clone());
        async move {
            if left == 0 {
                return None;
            }
            if cancel.load(Ordering::Relaxed) {
                return Some((Err(std::io::Error::other("cancelled")), (None, 0)));
            }
            let mut f = match f {
                Some(f) => f,
                None => {
                    let r = async {
                        let mut f = tokio::fs::File::open(&path).await?;
                        tokio::io::AsyncSeekExt::seek(&mut f, std::io::SeekFrom::Start(offset)).await?;
                        Ok::<_, std::io::Error>(f)
                    }
                    .await;
                    match r {
                        Ok(f) => f,
                        Err(e) => return Some((Err(e), (None, 0))),
                    }
                }
            };
            let mut buf = vec![0u8; (1 << 20).min(left as usize)];
            match f.read_exact(&mut buf).await {
                Ok(_) => {
                    done.fetch_add(buf.len() as u64, Ordering::Relaxed);
                    let n = buf.len() as u64;
                    Some((Ok(buf), (Some(f), left - n)))
                }
                Err(e) => Some((Err(e), (None, 0))),
            }
        }
    })
}

// ---------- Nextcloud sign-in (Login Flow v2) ----------

/// Set by `cancel_login`; checked by every browser sign-in that is waiting (Nextcloud, Box).
pub(crate) static LOGIN_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Signs in to a Nextcloud server through the browser and saves the account.
///
/// Opens the server's Login Flow v2 page; once the user approves there, Nextcloud hands back a
/// login name and a fresh app password, which go straight into the keychain (the UI never sees
/// them). `id` = the account to update (re-sign-in), or none for a new one. Returns the settings.
#[tauri::command]
pub async fn nextcloud_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
    server: String,
    id: Option<String>,
    name: String,
) -> Result<crate::config::Settings, String> {
    use tauri_plugin_opener::OpenerExt;
    let server = server_of(&server);
    if server.is_empty() {
        return Err("Enter your Nextcloud address first".into());
    }
    LOGIN_CANCEL.store(false, Ordering::Relaxed);
    let http = http();
    let r = http.post(format!("{server}/index.php/login/v2")).send().await.map_err(net_err)?;
    let v: serde_json::Value =
        ok(r).await?.json().await.map_err(|_| "That address doesn't look like a Nextcloud server".to_string())?;
    let (Some(login), Some(token), Some(poll)) =
        (v["login"].as_str(), v["poll"]["token"].as_str(), v["poll"]["endpoint"].as_str())
    else {
        return Err("That address doesn't look like a Nextcloud server".into());
    };
    app.opener().open_url(login, None::<&str>).map_err(|e| e.to_string())?;

    let deadline = std::time::Instant::now() + Duration::from_secs(20 * 60);
    let granted: serde_json::Value = loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if LOGIN_CANCEL.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled".into());
        }
        if std::time::Instant::now() > deadline {
            return Err("Sign-in timed out. Try again.".into());
        }
        match http.post(poll).form_body(&[("token", token)]).send().await {
            Ok(r) if r.status() == StatusCode::OK => break r.json().await.map_err(net_err)?,
            Ok(r) if r.status() == StatusCode::NOT_FOUND => continue, // not approved yet
            Ok(r) => {
                ok(r).await?;
                return Err("Nextcloud answered the sign-in in an unexpected way".into());
            }
            Err(_) => continue, // a network blip while waiting
        }
    };
    let (Some(srv), Some(user), Some(pass)) =
        (granted["server"].as_str(), granted["loginName"].as_str(), granted["appPassword"].as_str())
    else {
        return Err("Nextcloud didn't return credentials".into());
    };
    let s = state.settings();
    let mut p = id.and_then(|id| s.profiles.iter().find(|p| p.id == id).cloned()).unwrap_or_default();
    p.kind = ProviderKind::Nextcloud;
    p.endpoint = srv.trim_end_matches('/').to_string();
    p.access_key = user.to_string();
    p.secret_key = pass.to_string();
    if !name.trim().is_empty() {
        p.name = name.trim().to_string();
    }
    Dav::new(&p).containers().await?; // make sure it works before saving
    crate::config::store_account(&app, &state, p)
}

/// Stops a browser sign-in (`nextcloud_login`, `box_login`) that is waiting.
#[tauri::command]
pub fn cancel_login() {
    LOGIN_CANCEL.store(true, Ordering::Relaxed);
}

// ---------- HTTP helpers ----------

/// `application/x-www-form-urlencoded` body (reqwest 0.13 puts `.form()` behind a feature).
fn form(pairs: &[(&str, &str)]) -> String {
    let e = |v: &str| utf8_percent_encode(v, percent_encoding::NON_ALPHANUMERIC).to_string();
    pairs.iter().map(|(k, v)| format!("{}={}", e(k), e(v))).collect::<Vec<_>>().join("&")
}

trait FormExt {
    fn form_body(self, pairs: &[(&str, &str)]) -> Self;
}

impl FormExt for RequestBuilder {
    fn form_body(self, pairs: &[(&str, &str)]) -> Self {
        self.header("Content-Type", "application/x-www-form-urlencoded").body(form(pairs))
    }
}

fn net_err(e: reqwest::Error) -> String {
    let msg = if e.is_connect() {
        "Can't reach the server (check the address and your connection)".to_string()
    } else if e.is_timeout() {
        "The server took too long to answer".to_string()
    } else {
        e.to_string()
    };
    // reqwest messages can carry the URL; never the password (it's in a header).
    msg
}

/// Passes 2xx/207 through; turns anything else into a readable error.
async fn ok(r: Response) -> Result<Response, String> {
    let st = r.status();
    if st.is_success() {
        return Ok(r);
    }
    let body = r.text().await.unwrap_or_default();
    // Nextcloud/sabre put the reason in <s:message>.
    let detail = body
        .split("<s:message>")
        .nth(1)
        .and_then(|s| s.split("</s:message>").next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let what = match st.as_u16() {
        401 => "Wrong user name or (app) password".into(),
        403 => "Not allowed".into(),
        404 => "Not found".into(),
        405 => "The server refused this operation".into(),
        409 => "The parent folder doesn't exist".into(),
        412 => "It changed on the server meanwhile".into(),
        423 => "It's locked on the server".into(),
        507 => "Not enough space on the server".into(),
        _ => st.canonical_reason().unwrap_or("Request failed").to_string(),
    };
    Err(match detail {
        Some(d) => format!("{what} (HTTP {}): {d}", st.as_u16()),
        None => format!("{what} (HTTP {})", st.as_u16()),
    })
}

/// Path part of an absolute URL or an already-relative href.
fn url_path(u: &str) -> &str {
    match u.find("://") {
        Some(i) => u[i + 3..].find('/').map(|j| &u[i + 3 + j..]).unwrap_or("/"),
        None => u,
    }
}

fn decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// `YYYY-MM-DD`, `days` from today (UTC).
fn date_in_days(days: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, m, d) = civil_from_days(((now / 86_400) + days) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Howard Hinnant's algorithm.
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 1123 (`Wed, 16 Oct 2026 10:01:02 GMT`) → Unix millis.
fn parse_http_date(s: &str) -> Option<i64> {
    let p: Vec<&str> = s.split_whitespace().collect();
    let (d, mon, y, t) = (p.get(1)?.parse::<u32>().ok()?, *p.get(2)?, p.get(3)?.parse::<i64>().ok()?, p.get(4)?);
    let m = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .iter()
        .position(|x| *x == mon)? as u32
        + 1;
    let hms: Vec<i64> = t.split(':').filter_map(|x| x.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    Some((days_from_civil(y, m, d) * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2]) * 1000)
}

// ---------- PROPFIND parsing ----------

#[derive(Default, Debug, PartialEq)]
struct Prop {
    href: String,
    dir: bool,
    size: u64,
    modified: Option<i64>,
    /// Nextcloud `oc:checksums`, e.g. `SHA1:… MD5:…` (only when a client sent them).
    checksums: String,
}

/// The best hash in Nextcloud's checksum list, as `sha1:…` / `md5:…`.
fn checksum(list: &str) -> Option<String> {
    let parts: Vec<(String, String)> = list
        .split_whitespace()
        .filter_map(|c| c.split_once(':'))
        .map(|(a, v)| (a.to_ascii_lowercase(), v.to_ascii_lowercase()))
        .collect();
    ["sha1", "md5"].iter().find_map(|want| parts.iter().find(|(a, _)| a == want).map(|(a, v)| format!("{a}:{v}")))
}

/// Reads a `207 Multi-Status` body (any namespace prefixes).
fn parse_multistatus(xml: &str) -> Vec<Prop> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let (mut out, mut cur, mut field, mut text) = (vec![], None::<Prop>, String::new(), String::new());
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                match name.as_str() {
                    "response" => cur = Some(Prop::default()),
                    "collection" => cur.iter_mut().for_each(|p| p.dir = true),
                    _ => {}
                }
                field = name;
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == "collection" {
                    cur.iter_mut().for_each(|p| p.dir = true);
                }
            }
            Ok(Event::Text(t)) => text.push_str(&t),
            Ok(Event::GeneralRef(g)) => {
                let named = match &*g {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => None,
                };
                if let Some(c) = named.or_else(|| g.resolve_char_ref().ok().flatten()) {
                    text.push(c);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name();
                if let Some(p) = cur.as_mut() {
                    match name.as_ref() {
                        "href" if field == "href" => p.href = text.clone(),
                        "getcontentlength" => p.size = text.trim().parse().unwrap_or(0),
                        "getlastmodified" => p.modified = parse_http_date(text.trim()),
                        "checksum" => {
                            p.checksums.push(' ');
                            p.checksums.push_str(text.trim());
                        }
                        _ => {}
                    }
                }
                if name.as_ref() == "response" {
                    out.extend(cur.take());
                }
                text.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nextcloud_multistatus() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns">
 <d:response><d:href>/remote.php/dav/files/dev/Photos/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getlastmodified>Wed, 16 Oct 2026 10:01:02 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
 <d:response><d:href>/remote.php/dav/files/dev/Photos/a%20%26%20b.jpg</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>1234</d:getcontentlength><d:getlastmodified>Thu, 01 Jan 1970 00:00:01 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
 <D:response xmlns:D="DAV:"><D:href>http://nas:5005/dav/x&amp;y.txt</D:href><D:propstat><D:prop><D:getcontentlength>7</D:getcontentlength><D:resourcetype></D:resourcetype></D:prop></D:propstat></D:response>
</d:multistatus>"#;
        let p = parse_multistatus(xml);
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].href.as_str(), p[0].dir), ("/remote.php/dav/files/dev/Photos/", true));
        assert_eq!(p[0].modified, Some(parse_http_date("Wed, 16 Oct 2026 10:01:02 GMT").unwrap()));
        assert_eq!((p[1].dir, p[1].size, p[1].modified), (false, 1234, Some(1000)));
        assert_eq!(decode(url_path(&p[1].href)), "/remote.php/dav/files/dev/Photos/a & b.jpg");
        assert_eq!((url_path(&p[2].href), p[2].size, p[2].dir), ("/dav/x&y.txt", 7, false));
    }

    /// End-to-end against a real Nextcloud. Skipped unless NOEXPLORER_TEST_NEXTCLOUD is set:
    /// `./scripts/dev-nextcloud.sh up && ./scripts/dev-nextcloud.sh seed`, then
    /// `NOEXPLORER_TEST_NEXTCLOUD=http://localhost:8080 NOEXPLORER_TEST_DAV_USER=dev
    ///  NOEXPLORER_TEST_DAV_PASS='NoExplorer-dev-2026!' cargo test nextcloud_roundtrip -- --nocapture`
    #[tokio::test]
    async fn nextcloud_roundtrip() {
        let Ok(server) = std::env::var("NOEXPLORER_TEST_NEXTCLOUD") else {
            eprintln!("skipping nextcloud_roundtrip: NOEXPLORER_TEST_NEXTCLOUD not set");
            return;
        };
        let p = Profile {
            id: "nc".into(),
            kind: ProviderKind::Nextcloud,
            endpoint: server,
            access_key: std::env::var("NOEXPLORER_TEST_DAV_USER").unwrap(),
            secret_key: std::env::var("NOEXPLORER_TEST_DAV_PASS").unwrap(),
            ..Default::default()
        };
        let d = Dav::new(&p);
        assert_eq!(d.containers().await.unwrap()[0].name, "Files");
        let root = d.list("Files", "").await.unwrap();
        assert!(
            root.iter().any(|e| e.dir && e.key == "Projects/"),
            "{:?}",
            root.iter().map(|e| &e.key).collect::<Vec<_>>()
        );
        let proj = d.list("Files", "Projects/").await.unwrap();
        assert!(proj.iter().any(|e| !e.dir && e.key == "Projects/readme.txt" && e.size == 21 && e.modified.is_some()));
        assert!(proj.iter().any(|e| e.dir && e.key == "Projects/sub folder/" && e.name == "sub folder"));
        let all = d.list_all("Files", "Projects/").await.unwrap();
        assert!(all.iter().any(|(k, _)| k == "Projects/sub folder/tiếng việt.txt"), "{all:?}");

        let dir = std::env::temp_dir().join(format!("nx-nc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..(40 * 1024 * 1024 + 7)).map(|i| (i * 13 % 251) as u8).collect();
        std::fs::write(dir.join("big.bin"), &big).unwrap();
        std::fs::write(dir.join("small.txt"), "small").unwrap();
        let job = Job::untracked("up", "t".into(), "nc", "Files", "");
        // Into folders that don't exist yet; big.bin goes through chunked upload v2.
        d.upload_file("Files", "rt/a b/big.bin", &dir.join("big.bin"), big.len() as u64, &job).await.unwrap();
        d.upload_file("Files", "rt/a b/c/small.txt", &dir.join("small.txt"), 5, &job).await.unwrap();
        let listed = d.list_all("Files", "rt/").await.unwrap();
        assert!(listed.contains(&("rt/a b/big.bin".into(), big.len() as u64)), "{listed:?}");
        d.download_file("Files", "rt/a b/big.bin", &dir.join("out.bin"), &job).await.unwrap();
        assert!(std::fs::read(dir.join("out.bin")).unwrap() == big, "download differs");
        assert_eq!(d.read_head("Files", "rt/a b/c/small.txt", 3).await.unwrap(), b"sma");
        let (part, total) = d.read_range("rt/a b/big.bin", 1000, Some(1009), 1 << 20).await.unwrap();
        assert_eq!((part.as_slice(), total), (&big[1000..1010], Some(big.len() as u64)));
        assert!(
            d.link("Files", "rt/a b/c/small.txt", 60, true).await.unwrap().contains("/nc/Files/rt/a%20b/c/small.txt")
        );
        let share = d.link("Files", "rt/a b/c/small.txt", 7 * 86_400, false).await.unwrap();
        assert!(share.contains("/s/"), "public share link expected, got {share}");
        d.mkdir("Files", "rt/", "new folder").await.unwrap();
        assert!(d.list("Files", "rt/").await.unwrap().iter().any(|e| e.key == "rt/new folder/"));
        assert_eq!(d.delete("Files", vec!["rt/".into()]).await.unwrap(), 1);
        assert!(d.list("Files", "rt/").await.is_err(), "rt/ should be gone");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn urls_dates_and_roots() {
        assert_eq!(enc_path("a b/c#d/"), "a%20b/c%23d/");
        assert_eq!(server_of("cloud.example.com/"), "https://cloud.example.com");
        assert_eq!(server_of("https://x.y/nc/remote.php/dav/files/me/"), "https://x.y/nc");
        let p = Profile {
            kind: ProviderKind::Nextcloud,
            endpoint: "https://x.y/nc".into(),
            access_key: "a b".into(),
            ..Default::default()
        };
        let d = Dav::new(&p);
        assert_eq!(d.root, "https://x.y/nc/remote.php/dav/files/a%20b/");
        assert_eq!(d.root_path, "/nc/remote.php/dav/files/a b/");
        assert_eq!(d.url("Docs/r é.txt"), "https://x.y/nc/remote.php/dav/files/a%20b/Docs/r%20%C3%A9.txt");
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 16)), (2026, 10, 16));
        assert_eq!(
            parse_http_date("Thu, 29 Feb 2024 23:59:59 GMT"),
            Some((days_from_civil(2024, 2, 29) * 86_400 + 86_399) * 1000)
        );
    }
}
