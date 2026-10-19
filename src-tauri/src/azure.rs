//! Azure Blob Storage provider, over its REST API with Shared Key auth.
//!
//! The account form maps to: `access_key` = storage account name, `secret_key` = account key
//! (base64), and an optional `endpoint`: empty means `https://<account>.blob.core.windows.net`
//! ([`blob_endpoint`]); fill it for other clouds, a custom domain or Azurite
//! (`http://127.0.0.1:10000/devstoreaccount1`). Containers are the buckets. Folders are `/` in blob names, like S3: listing
//! uses `delimiter=/`, and an empty folder is a zero-length `name/` marker blob.
//!
//! Uploads: one Put Blob up to [`SINGLE_PUT_MAX`], else blocks of [`BLOCK`] sent in parallel and
//! committed with Put Block List. Downloads continue `.part` files (`x-ms-range`). Links are
//! service SAS URLs. Moves and copies run on the server (Copy Blob), never over an existing name.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use futures::{StreamExt, TryStreamExt, stream};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::config::{Profile, normalize_endpoint};
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry, drop_part, open_part, part_path, resume_offset};
use crate::util::file_name;
use crate::webdav::{civil_from_days, enc, enc_path, file_stream, net_err, parse_http_date};

const VERSION: &str = "2023-11-03";
/// Files up to this size go up in one request.
const SINGLE_PUT_MAX: u64 = 64 * 1024 * 1024;
/// Block size for bigger files (Azure allows up to 50,000 blocks per blob).
const BLOCK: u64 = 8 * 1024 * 1024;
const BLOCK_CONCURRENCY: usize = 4;

pub struct Azure {
    profile: Profile,
    http: reqwest::Client,
    /// `https://acct.blob.core.windows.net` or `http://127.0.0.1:10000/devstoreaccount1`, no slash.
    base: String,
    key: Vec<u8>,
}

/// The Blob endpoint: the one entered, else the storage account's public one.
pub fn blob_endpoint(p: &Profile) -> String {
    let e = normalize_endpoint(&p.endpoint);
    if e.is_empty() {
        format!("https://{}.blob.core.windows.net", p.access_key.trim().to_ascii_lowercase())
    } else {
        e.trim_end_matches('/').to_string()
    }
}

impl Azure {
    pub fn new(p: &Profile) -> Azure {
        let base = blob_endpoint(p);
        let key = B64.decode(p.secret_key.trim()).unwrap_or_default();
        let http = if p.cert_pin.is_empty() {
            crate::webdav::http()
        } else {
            crate::tls::pinned_client(&p.cert_pin).unwrap_or_else(|_| crate::webdav::http())
        };
        Azure { profile: p.clone(), http, base, key }
    }

    pub fn account(&self) -> &str {
        &self.profile.id
    }

    fn name(&self) -> &str {
        self.profile.access_key.trim()
    }

    fn url(&self, container: &str, key: &str) -> String {
        if key.is_empty() {
            format!("{}/{}", self.base, enc(container))
        } else {
            format!("{}/{}/{}", self.base, enc(container), enc_path(key))
        }
    }

    /// A signed request. `len` = the body's length (Content-Length in the signature); `headers` are
    /// extra `x-ms-*` (and Content-Type) headers.
    fn req(&self, m: Method, url: &str, len: Option<u64>, headers: &[(&str, String)]) -> RequestBuilder {
        let date = http_date(SystemTime::now());
        let mut ms: BTreeMap<String, String> = BTreeMap::new();
        ms.insert("x-ms-date".into(), date);
        ms.insert("x-ms-version".into(), VERSION.into());
        let mut content_type = String::new();
        for (k, v) in headers {
            let k = k.to_ascii_lowercase();
            if k == "content-type" {
                content_type = v.clone();
            } else {
                ms.insert(k, v.clone());
            }
        }
        let parsed = reqwest::Url::parse(url).expect("valid URL");
        let mut resource = format!("/{}{}", self.name(), parsed.path());
        let mut q: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (k, v) in parsed.query_pairs() {
            q.entry(k.to_ascii_lowercase()).or_default().push(v.into_owned());
        }
        for (k, mut vs) in q {
            vs.sort();
            resource.push_str(&format!("\n{k}:{}", vs.join(",")));
        }
        let length = len.filter(|n| *n > 0).map(|n| n.to_string()).unwrap_or_default();
        let canon_headers: String = ms.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
        let to_sign = format!("{m}\n\n\n{length}\n\n{content_type}\n\n\n\n\n\n\n{canon_headers}{resource}");
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC takes any key length");
        mac.update(to_sign.as_bytes());
        let sig = B64.encode(mac.finalize().into_bytes());
        let mut b = self.http.request(m, url).header("Authorization", format!("SharedKey {}:{sig}", self.name()));
        for (k, v) in &ms {
            b = b.header(k, v);
        }
        if !content_type.is_empty() {
            b = b.header("Content-Type", content_type);
        }
        if let Some(n) = len {
            b = b.header("Content-Length", n);
        }
        b
    }

    async fn send(&self, b: RequestBuilder) -> Result<Response, String> {
        ok(b.send().await.map_err(net_err)?).await
    }

    /// Creates container `name` (experimental "create_buckets"), private (no public access).
    pub async fn create_container(&self, name: &str) -> Result<(), String> {
        self.send(self.req(Method::PUT, &format!("{}?restype=container", self.url(name, "")), Some(0), &[])).await?;
        Ok(())
    }

    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        let mut out = vec![];
        let mut marker = String::new();
        loop {
            let url = format!("{}/?comp=list&marker={}", self.base, q(&marker));
            let xml = self.send(self.req(Method::GET, &url, None, &[])).await?.text().await.map_err(net_err)?;
            let page = parse_list(&xml);
            out.extend(page.containers.into_iter().map(|name| BucketInfo { name, region: None }));
            match page.next {
                Some(m) if !m.is_empty() => marker = m,
                _ => return Ok(out),
            }
        }
    }

    /// One folder level (`flat` = everything below, no folders: Compare and deletes).
    async fn list_raw(&self, container: &str, prefix: &str, flat: bool) -> Result<Vec<Entry>, String> {
        let mut out = vec![];
        let mut marker = String::new();
        loop {
            let delim = if flat { "" } else { "&delimiter=%2F" };
            let url = format!(
                "{}?restype=container&comp=list&prefix={}{delim}&marker={}",
                self.url(container, ""),
                q(prefix),
                q(&marker)
            );
            let xml = self.send(self.req(Method::GET, &url, None, &[])).await?.text().await.map_err(net_err)?;
            let page = parse_list(&xml);
            for p in page.prefixes {
                out.push(Entry { name: file_name(&p), key: p, size: 0, modified: None, dir: true, hash: None });
            }
            for b in page.blobs {
                if b.name == prefix || (flat && b.name.ends_with('/')) {
                    continue; // the folder's own marker (and, flat, other markers)
                }
                out.push(Entry {
                    name: file_name(&b.name),
                    key: b.name,
                    size: b.size,
                    modified: b.modified,
                    dir: false,
                    hash: b.md5.and_then(|m| B64.decode(m).ok()).map(|d| format!("md5:{}", hex(&d))),
                });
            }
            match page.next {
                Some(m) if !m.is_empty() => marker = m,
                _ => return Ok(out),
            }
        }
    }

    pub async fn list(&self, container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        self.list_raw(container, prefix, false).await
    }

    pub async fn list_files(&self, container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        self.list_raw(container, prefix, true).await
    }

    pub async fn list_all(&self, container: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        Ok(self.list_files(container, prefix).await?.into_iter().map(|e| (e.key, e.size)).collect())
    }

    /// A service SAS link, read-only, valid `secs` seconds.
    pub async fn link(&self, container: &str, key: &str, secs: u64, inline: bool) -> Result<String, String> {
        let expiry = iso_date(SystemTime::now() + Duration::from_secs(secs));
        let disposition = if inline { String::new() } else { format!("attachment; filename=\"{}\"", file_name(key)) };
        // Previews: no-store, or the webview keeps every previewed file on disk.
        let cache = if inline { "no-store" } else { "" };
        let resource = format!("/blob/{}/{container}/{key}", self.name());
        let to_sign = format!("r\n\n{expiry}\n{resource}\n\n\n\n{VERSION}\nb\n\n\n{cache}\n{disposition}\n\n\n");
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC takes any key length");
        mac.update(to_sign.as_bytes());
        let sig = B64.encode(mac.finalize().into_bytes());
        let mut url = format!("{}?sv={VERSION}&sr=b&sp=r&se={}&sig={}", self.url(container, key), q(&expiry), q(&sig));
        if !cache.is_empty() {
            url.push_str(&format!("&rscc={}", q(cache)));
        }
        if !disposition.is_empty() {
            url.push_str(&format!("&rscd={}", q(&disposition)));
        }
        Ok(url)
    }

    pub async fn read_range(
        &self,
        container: &str,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        let last = end.unwrap_or(start + cap - 1).min(start + cap - 1);
        let r = self
            .send(self.req(
                Method::GET,
                &self.url(container, key),
                None,
                &[("x-ms-range", format!("bytes={start}-{last}"))],
            ))
            .await?;
        let total = r
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|n| n.parse().ok());
        let mut bytes = r.bytes().await.map_err(net_err)?.to_vec();
        bytes.truncate(cap as usize);
        Ok((bytes, total))
    }

    pub async fn read_head(&self, container: &str, key: &str, max: u64) -> Result<Vec<u8>, String> {
        Ok(self.read_range(container, key, 0, Some(max.max(1) - 1), max.max(1)).await?.0)
    }

    pub async fn mkdir(&self, container: &str, prefix: &str, name: &str) -> Result<(), String> {
        let url = self.url(container, &format!("{prefix}{name}/"));
        self.send(self.req(Method::PUT, &url, Some(0), &[("x-ms-blob-type", "BlockBlob".into())])).await.map(|_| ())
    }

    async fn exists(&self, container: &str, key: &str) -> Result<bool, String> {
        if key.ends_with('/') {
            let url = format!("{}?restype=container&comp=list&prefix={}&maxresults=1", self.url(container, ""), q(key));
            let xml = self.send(self.req(Method::GET, &url, None, &[])).await?.text().await.map_err(net_err)?;
            let p = parse_list(&xml);
            return Ok(!p.blobs.is_empty() || !p.prefixes.is_empty());
        }
        let r = self.req(Method::HEAD, &self.url(container, key), None, &[]).send().await.map_err(net_err)?;
        Ok(r.status().is_success())
    }

    async fn delete_blob(&self, container: &str, key: &str) -> Result<(), String> {
        let r = self.req(Method::DELETE, &self.url(container, key), None, &[]).send().await.map_err(net_err)?;
        if r.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        ok(r).await.map(|_| ())
    }

    pub async fn delete(&self, container: &str, keys: Vec<String>, job: Option<&Job>) -> Result<usize, String> {
        let mut all = vec![];
        for k in keys {
            if k.ends_with('/') {
                all.extend(self.list_files(container, &k).await?.into_iter().map(|e| e.key));
            }
            all.push(k); // the file itself, or the folder's marker blob (a missing one is fine)
        }
        if let Some(j) = job {
            j.files_total.store(all.len() as u64, Ordering::Relaxed);
        }
        let n = all.len();
        stream::iter(all)
            .map(|k| async move {
                if job.is_some_and(|j| j.cancelled()) {
                    return Err("cancelled".to_string());
                }
                self.delete_blob(container, &k).await?;
                if let Some(j) = job {
                    j.files_done.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            })
            .buffer_unordered(8)
            .try_collect::<Vec<()>>()
            .await?;
        Ok(n)
    }

    /// Server-side copy (Copy Blob) of one blob, waiting until Azure has finished it.
    async fn copy_blob(&self, container: &str, from: &str, to: &str) -> Result<(), String> {
        let src = self.url(container, from);
        let r =
            self.send(self.req(Method::PUT, &self.url(container, to), Some(0), &[("x-ms-copy-source", src)])).await?;
        let mut status = header(&r, "x-ms-copy-status");
        while status == "pending" {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let h = self.send(self.req(Method::HEAD, &self.url(container, to), None, &[])).await?;
            status = header(&h, "x-ms-copy-status");
        }
        if status == "success" || status.is_empty() { Ok(()) } else { Err(format!("Copy {status}")) }
    }

    /// Copies or moves (`keep` = copy) a blob or a folder to `to`. Never over an existing name.
    pub async fn transfer_within(
        &self,
        container: &str,
        from: &str,
        to: &str,
        keep: bool,
        job: Option<&Job>,
    ) -> Result<(), String> {
        if self.exists(container, to).await? {
            return Err(format!("“{}” already exists there", file_name(to)));
        }
        let mut keys: Vec<String> = if from.ends_with('/') {
            self.list_files(container, from).await?.into_iter().map(|e| e.key).collect()
        } else {
            vec![from.to_string()]
        };
        if from.ends_with('/') && self.exists(container, from).await? && keys.is_empty() {
            keys.push(from.to_string()); // an empty folder: just its marker
        }
        if let Some(j) = job {
            j.files_total.store(keys.len() as u64, Ordering::Relaxed);
        }
        for k in &keys {
            if job.is_some_and(|j| j.cancelled()) {
                return Err("cancelled".into());
            }
            self.copy_blob(container, k, &format!("{to}{}", &k[from.len()..])).await?;
            if let Some(j) = job {
                j.files_done.fetch_add(1, Ordering::Relaxed);
            }
        }
        if !keep {
            self.delete(container, vec![from.to_string()], None).await?;
        }
        Ok(())
    }

    pub async fn upload_file(
        &self,
        container: &str,
        key: &str,
        path: &Path,
        size: u64,
        job: &Job,
    ) -> Result<(), String> {
        let ct = crate::util::upload_mime(key, path);
        let url = self.url(container, key);
        if size <= SINGLE_PUT_MAX {
            let body = reqwest::Body::wrap_stream(file_stream(path.to_path_buf(), 0, size, job));
            let b = self.req(
                Method::PUT,
                &url,
                Some(size),
                &[("x-ms-blob-type", "BlockBlob".into()), ("Content-Type", ct)],
            );
            return self.send(b.body(body)).await.map(|_| ());
        }
        let blocks = size.div_ceil(BLOCK);
        // 6 digits → 8 base64 characters with no "=" padding (padding in a query string trips some
        // servers' signature checks).
        let id = |i: u64| B64.encode(format!("{i:06}"));
        stream::iter(0..blocks)
            .map(|i| {
                let url = format!("{url}?comp=block&blockid={}", q(&id(i)));
                async move {
                    if job.cancelled() {
                        return Err("cancelled".to_string());
                    }
                    let offset = i * BLOCK;
                    let len = BLOCK.min(size - offset);
                    job.gate(len).await;
                    let mut f = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
                    f.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| e.to_string())?;
                    let mut buf = vec![0u8; len as usize];
                    f.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
                    self.send(self.req(Method::PUT, &url, Some(len), &[]).body(buf)).await?;
                    job.bytes_done.fetch_add(len, Ordering::Relaxed);
                    Ok(())
                }
            })
            .buffer_unordered(BLOCK_CONCURRENCY)
            .try_collect::<Vec<()>>()
            .await?;
        let list: String = (0..blocks).map(|i| format!("<Latest>{}</Latest>", id(i))).collect();
        let xml = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>{list}</BlockList>");
        let b = self.req(
            Method::PUT,
            &format!("{url}?comp=blocklist"),
            Some(xml.len() as u64),
            &[("x-ms-blob-content-type", ct)],
        );
        self.send(b.body(xml)).await.map(|_| ())
    }

    pub async fn download_file(&self, container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let tmp = part_path(path);
        let offset = resume_offset(&tmp, job).await;
        let extra: Vec<(&str, String)> =
            if offset > 0 { vec![("x-ms-range", format!("bytes={offset}-"))] } else { vec![] };
        let r = self.req(Method::GET, &self.url(container, key), None, &extra).send().await.map_err(net_err)?;
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

fn header(r: &Response, name: &str) -> String {
    r.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_ascii_lowercase()
}

/// Azure's error body carries `<Code>` and `<Message>`; show those, in plain words where we can.
async fn ok(r: Response) -> Result<Response, String> {
    let st = r.status();
    if st.is_success() {
        return Ok(r);
    }
    let url = r.url().to_string();
    let body = r.text().await.unwrap_or_default();
    if st.as_u16() != 404 {
        crate::diag::log("azure", &format!("HTTP {} {url} {body}", st.as_u16()));
    }
    let tag =
        |t: &str| body.split(&format!("<{t}>")).nth(1).and_then(|s| s.split(&format!("</{t}>")).next()).map(str::trim);
    let what = match st.as_u16() {
        403 => "Access denied: check the account name and key".to_string(),
        404 => "Not found".to_string(),
        409 => "It already exists, or the container is busy".to_string(),
        _ => st.canonical_reason().unwrap_or("Request failed").to_string(),
    };
    Err(match (tag("Code"), tag("Message")) {
        (Some(c), Some(m)) => format!("{what} ({c}): {}", m.lines().next().unwrap_or(m)),
        (Some(c), None) => format!("{what} ({c})"),
        _ => format!("{what} (HTTP {})", st.as_u16()),
    })
}

/// A query-string value: everything but letters and digits percent-encoded (base64 has + / =).
fn q(v: &str) -> String {
    percent_encoding::utf8_percent_encode(v, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `Sun, 18 Oct 2026 10:01:02 GMT` (x-ms-date).
fn http_date(t: SystemTime) -> String {
    let s = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let (y, m, d) = civil_from_days(s.div_euclid(86_400));
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(s.div_euclid(86_400)).rem_euclid(7) as usize];
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][m as usize - 1];
    let r = s.rem_euclid(86_400);
    format!("{wd}, {d:02} {mon} {y} {:02}:{:02}:{:02} GMT", r / 3600, r / 60 % 60, r % 60)
}

/// `2026-10-18T10:01:02Z` (SAS expiry).
fn iso_date(t: SystemTime) -> String {
    let s = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let (y, m, d) = civil_from_days(s.div_euclid(86_400));
    let r = s.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", r / 3600, r / 60 % 60, r % 60)
}

// ---------- listing XML ----------

#[derive(Default)]
struct Blob {
    name: String,
    size: u64,
    modified: Option<i64>,
    md5: Option<String>,
}

#[derive(Default)]
struct Page {
    containers: Vec<String>,
    blobs: Vec<Blob>,
    prefixes: Vec<String>,
    next: Option<String>,
}

/// Parses List Containers / List Blobs results (only the fields we use).
fn parse_list(xml: &str) -> Page {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    // No trimming: it trims each text piece on its own, so "x &amp; y" would lose its spaces. Text
    // is cleared at every tag, so whitespace between elements never reaches a value.
    let mut page = Page::default();
    let mut path: Vec<String> = vec![];
    let mut blob: Option<Blob> = None;
    let mut text = String::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let n = e.local_name().as_ref().to_string();
                if n == "Blob" {
                    blob = Some(Blob::default());
                }
                path.push(n);
                text.clear();
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
            Ok(Event::End(_)) => {
                let n = path.pop().unwrap_or_default();
                let parent = path.last().map(String::as_str).unwrap_or("");
                match (parent, n.as_str()) {
                    ("Container", "Name") => page.containers.push(text.clone()),
                    ("BlobPrefix", "Name") => page.prefixes.push(text.clone()),
                    ("Blob", "Name") => {
                        if let Some(b) = blob.as_mut() {
                            b.name = text.clone()
                        }
                    }
                    ("Properties", "Content-Length") => {
                        if let Some(b) = blob.as_mut() {
                            b.size = text.parse().unwrap_or(0)
                        }
                    }
                    ("Properties", "Last-Modified") => {
                        if let Some(b) = blob.as_mut() {
                            b.modified = parse_http_date(&text)
                        }
                    }
                    ("Properties", "Content-MD5") if !text.is_empty() => {
                        if let Some(b) = blob.as_mut() {
                            b.md5 = Some(text.clone())
                        }
                    }
                    ("EnumerationResults", "NextMarker") => page.next = Some(text.clone()),
                    (_, "Blob") => {
                        if let Some(b) = blob.take() {
                            page.blobs.push(b)
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_defaults_to_the_account() {
        let p = Profile { access_key: " MyStore ".into(), ..Default::default() };
        assert_eq!(blob_endpoint(&p), "https://mystore.blob.core.windows.net");
        let p = Profile { endpoint: "http://127.0.0.1:10000/devstoreaccount1/".into(), ..p };
        assert_eq!(blob_endpoint(&p), "http://127.0.0.1:10000/devstoreaccount1");
    }

    #[test]
    fn parses_list_blobs() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><EnumerationResults ContainerName="c"><Prefix>a/</Prefix><Blobs>
          <Blob><Name>a/x &amp; y.txt</Name><Properties><Last-Modified>Sun, 18 Oct 2026 10:01:02 GMT</Last-Modified><Content-Length>5</Content-Length><Content-MD5>XUFAKrxLKna5cZ2REBfFkg==</Content-MD5></Properties></Blob>
          <BlobPrefix><Name>a/sub/</Name></BlobPrefix></Blobs><NextMarker>m2</NextMarker></EnumerationResults>"#;
        let p = parse_list(xml);
        assert_eq!(p.blobs.len(), 1);
        assert_eq!(p.blobs[0].name, "a/x & y.txt");
        assert_eq!(p.blobs[0].size, 5);
        assert!(p.blobs[0].modified.is_some());
        assert_eq!(p.prefixes, vec!["a/sub/".to_string()]);
        assert_eq!(p.next.as_deref(), Some("m2"));
    }

    #[test]
    fn dates() {
        let t = UNIX_EPOCH + Duration::from_secs(1_792_000_000);
        assert_eq!(http_date(t), "Wed, 14 Oct 2026 17:46:40 GMT");
        assert_eq!(iso_date(t), "2026-10-14T17:46:40Z");
    }

    /// Against Azurite (`docker run -p 10000:10000 mcr.microsoft.com/azure-storage/azurite azurite-blob
    /// --blobHost 0.0.0.0 --skipApiVersionCheck`) with `NOEXPLORER_TEST_AZURITE=http://127.0.0.1:10000/devstoreaccount1`.
    #[tokio::test]
    async fn azurite_roundtrip() {
        let Ok(endpoint) = std::env::var("NOEXPLORER_TEST_AZURITE") else {
            eprintln!("skipping azurite_roundtrip: NOEXPLORER_TEST_AZURITE not set");
            return;
        };
        let p = Profile {
            kind: crate::config::ProviderKind::Azure,
            endpoint,
            access_key: "devstoreaccount1".into(),
            // Azurite's well-known development key.
            secret_key: "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw=="
                .into(),
            ..Default::default()
        };
        let az = Azure::new(&p);
        let c = "noexplorer-test";
        let _ = az.send(az.req(Method::PUT, &format!("{}?restype=container", az.url(c, "")), Some(0), &[])).await;
        assert!(az.containers().await.unwrap().iter().any(|b| b.name == c));
        // New container through the provider interface (name checked first), then gone again.
        let made = format!("nx-new-{}", std::process::id());
        let remote = crate::provider::Remote::new(&p);
        assert!(remote.create_container("Bad_Name").await.is_err());
        remote.create_container(&made).await.unwrap();
        assert!(remote.containers().await.unwrap().iter().any(|b| b.name == made));
        assert!(remote.create_container(&made).await.is_err(), "already exists");
        az.send(az.req(Method::DELETE, &format!("{}?restype=container", az.url(&made, "")), Some(0), &[]))
            .await
            .unwrap();

        let dir = std::env::temp_dir().join(format!("nx-az-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..(SINGLE_PUT_MAX + 3 * 1024 * 1024)).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(dir.join("big.bin"), &big).unwrap();
        std::fs::write(dir.join("a b.txt"), "hello azure").unwrap();
        let job = Job::untracked("up", "t".into(), "", c, "");
        az.upload_file(c, "rt/a b.txt", &dir.join("a b.txt"), 11, &job).await.unwrap();
        az.upload_file(c, "rt/big.bin", &dir.join("big.bin"), big.len() as u64, &job).await.unwrap(); // blocks
        az.mkdir(c, "rt/", "empty").await.unwrap();

        let level = az.list(c, "rt/").await.unwrap();
        assert!(level.iter().any(|e| e.key == "rt/a b.txt" && e.size == 11));
        assert!(level.iter().any(|e| e.dir && e.key == "rt/empty/"));
        assert_eq!(az.read_head(c, "rt/a b.txt", 5).await.unwrap(), b"hello");
        let files = az.list_files(c, "rt/").await.unwrap();
        assert!(
            files.iter().any(|e| e.key == "rt/a b.txt" && e.hash.as_deref().is_some_and(|h| h.starts_with("md5:")))
        );

        az.download_file(c, "rt/big.bin", &dir.join("out.bin"), &job).await.unwrap();
        assert!(std::fs::read(dir.join("out.bin")).unwrap() == big, "download differs");

        // A SAS link works without credentials.
        let link = az.link(c, "rt/a b.txt", 600, true).await.unwrap();
        let r = reqwest::get(&link).await.unwrap();
        assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
        assert_eq!(r.text().await.unwrap(), "hello azure");

        // Server-side move, copy, and never over an existing name.
        az.transfer_within(c, "rt/a b.txt", "rt/moved.txt", false, None).await.unwrap();
        az.transfer_within(c, "rt/moved.txt", "rt/copy.txt", true, None).await.unwrap();
        assert!(az.transfer_within(c, "rt/moved.txt", "rt/copy.txt", true, None).await.is_err());
        let names: Vec<String> = az.list(c, "rt/").await.unwrap().into_iter().map(|e| e.name).collect();
        assert!(names.contains(&"moved.txt".into()) && names.contains(&"copy.txt".into()));
        assert!(!names.contains(&"a b.txt".into()));

        az.delete(c, vec!["rt/".into()], None).await.unwrap();
        assert!(az.list_files(c, "rt/").await.unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
