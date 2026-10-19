//! S3 provider: client construction and every S3 operation behind `provider::Remote` (ADR-025).
//!
//! - Path-style addressing per profile (default on, ADR-007); checksums only when required (ADR-004).
//! - AWS: each bucket is used in its own region (ADR-017). The region is learned from ListBuckets or
//!   one HEAD Bucket, cached per account, and gets its own client.
//! - Folders are virtual: a "folder" is a common prefix ending in `/` (listed with
//!   `delimiter("/")`), optionally backed by an empty `prefix/` marker object (what `mkdir` creates).
//! - Uploads above [`MULTIPART_THRESHOLD`] go multipart with [`PART_CONCURRENCY`] parts in flight
//!   (ADR-005); downloads stream to `<name>.part` and are renamed on success (ADR-006).

use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{
    BehaviorVersion, Builder, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    BucketLocationConstraint, CompletedMultipartUpload, CompletedPart, CreateBucketConfiguration, Delete,
    ObjectIdentifier,
};
use futures::{StreamExt, TryStreamExt, stream};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::config::{Profile, effective_region, is_aws, normalize_endpoint};
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry, drop_part, open_part, part_path, resume_offset};
use crate::util::{err, file_name, is_text_like};

pub const MULTIPART_THRESHOLD: u64 = 16 * 1024 * 1024;
pub const MIN_PART_SIZE: u64 = 8 * 1024 * 1024;
/// S3 allows 10 000 parts; stay well below so huge files still fit.
pub(crate) const MAX_PARTS: u64 = 9_000;
pub const PART_CONCURRENCY: usize = 4;

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
    if is_gcs(&endpoint) {
        b = b.interceptor(Gcs { project: p.project.trim().to_string() });
    }
    if !endpoint.is_empty() && !is_aws(p) {
        b = b.endpoint_url(endpoint);
    }
    // A pinned self-signed certificate (experimental): requests go through reqwest (tls.rs).
    if !p.cert_pin.is_empty()
        && !is_aws(p)
        && let Ok(h) = crate::tls::PinnedHttpClient::new(&p.cert_pin)
    {
        b = b.http_client(h);
    }
    Client::from_conf(b.build())
}

/// Google Cloud Storage's XML API (HMAC keys) speaks S3 with two differences, handled before signing:
/// - it rejects query parameters it doesn't know with "Invalid argument", and the SDK tags many
///   requests with `x-id=<Operation>` (ListBuckets, GetObject, PutObject, UploadPart…), purely
///   informational, so it's removed;
/// - listing buckets needs the project ("Project number must be specified") unless the key's account
///   has a default project for interoperable access. It goes in `x-amz-project-id`: Google maps
///   `x-amz-*` to its `x-goog-*` headers, and refuses a request that mixes the two families
///   (`ExcessHeaderValues`), which SigV4's own `x-amz-date` would do with `x-goog-project-id`.
fn is_gcs(endpoint: &str) -> bool {
    endpoint
        .split("://")
        .nth(1)
        .unwrap_or(endpoint)
        .split(['/', ':'])
        .next()
        .is_some_and(|h| h.ends_with("storage.googleapis.com"))
}

#[derive(Debug)]
struct Gcs {
    project: String,
}

impl aws_smithy_runtime_api::client::interceptors::Intercept for Gcs {
    fn name(&self) -> &'static str {
        "Gcs"
    }

    fn modify_before_signing(
        &self,
        ctx: &mut aws_smithy_runtime_api::client::interceptors::context::BeforeTransmitInterceptorContextMut<'_>,
        _rc: &aws_smithy_runtime_api::client::runtime_components::RuntimeComponents,
        _cfg: &mut aws_smithy_types::config_bag::ConfigBag,
    ) -> Result<(), aws_smithy_runtime_api::box_error::BoxError> {
        let uri = without_x_id(ctx.request().uri());
        // ListBuckets (the service root) and CreateBucket (PUT on a bare bucket) need the project.
        let needs = is_service_root(&uri) || (ctx.request().method() == "PUT" && is_bare_bucket(&uri));
        ctx.request_mut().set_uri(uri)?;
        if needs && !self.project.is_empty() {
            ctx.request_mut().headers_mut().insert("x-amz-project-id", self.project.clone());
        }
        Ok(())
    }
}

/// ListBuckets is the only request on the service root (`https://host/` with no bucket).
fn is_service_root(uri: &str) -> bool {
    let rest = uri.split("://").nth(1).unwrap_or(uri);
    rest.split_once('/').is_none_or(|(_, p)| p.is_empty() || p.starts_with('?'))
}

/// `https://host/bucket` with no key and no query (path-style): what CreateBucket looks like.
fn is_bare_bucket(uri: &str) -> bool {
    let rest = uri.split("://").nth(1).unwrap_or(uri);
    rest.split_once('/').is_some_and(|(_, p)| {
        let p = p.trim_end_matches('/');
        !p.is_empty() && !p.contains('/') && !p.contains('?')
    })
}

/// `…?x-id=GetObject&a=b` → `…?a=b` (and `…?x-id=X` → `…`).
fn without_x_id(uri: &str) -> String {
    let Some((base, q)) = uri.split_once('?') else { return uri.to_string() };
    let kept: Vec<&str> = q.split('&').filter(|p| !p.starts_with("x-id=") && *p != "x-id").collect();
    if kept.is_empty() { base.to_string() } else { format!("{base}?{}", kept.join("&")) }
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

/// One S3 account: its default client plus, on AWS, per-region clients for its buckets.
pub struct S3 {
    profile: Profile,
    base: Client,
    /// AWS only: bucket → region, learned from ListBuckets or HEAD Bucket.
    regions: Mutex<HashMap<String, String>>,
    /// AWS only: clients for buckets outside the profile's region, keyed by region.
    regional: Mutex<HashMap<String, Client>>,
}

impl S3 {
    /// The profile id of this account.
    pub fn account(&self) -> &str {
        &self.profile.id
    }

    pub fn new(p: &Profile) -> S3 {
        S3 {
            profile: p.clone(),
            base: make_client(p),
            regions: Mutex::new(HashMap::new()),
            regional: Mutex::new(HashMap::new()),
        }
    }

    /// Client to use for `bucket`. On AWS each bucket must be addressed in its own region
    /// (signing with another region fails), so the region is detected once and cached.
    /// Other providers have one endpoint: always the default client.
    pub async fn client_for(&self, bucket: &str) -> Client {
        let p = &self.profile;
        if !is_aws(p) || bucket.is_empty() {
            return self.base.clone();
        }
        let cached = self.regions.lock().unwrap().get(bucket).cloned();
        let region = match cached {
            Some(r) => r,
            None => {
                let r = detect_bucket_region(p, bucket).await.unwrap_or_else(|| effective_region(p));
                self.regions.lock().unwrap().insert(bucket.to_string(), r.clone());
                r
            }
        };
        if region == effective_region(p) {
            return self.base.clone();
        }
        let mut regional = self.regional.lock().unwrap();
        regional.entry(region.clone()).or_insert_with(|| make_client_in(p, &region)).clone()
    }

    /// Creates bucket `name` (experimental). On AWS it goes in the account's region, which outside
    /// us-east-1 has to be named in the request.
    pub async fn create_bucket(&self, name: &str) -> Result<(), String> {
        let p = &self.profile;
        let mut req = self.base.create_bucket().bucket(name);
        let region = effective_region(p);
        if is_aws(p) && region != "us-east-1" {
            let cfg = CreateBucketConfiguration::builder()
                .location_constraint(BucketLocationConstraint::from(region.as_str()))
                .build();
            req = req.create_bucket_configuration(cfg);
        }
        req.send().await.map_err(err)?;
        if is_aws(p) {
            self.regions.lock().unwrap().insert(name.to_string(), region);
        }
        Ok(())
    }

    /// Buckets visible to these credentials (+ region when the server reports it, as AWS does).
    /// Falls back to the profile's bucket when the key may not ListAllMyBuckets.
    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        let p = &self.profile;
        let out = match self.base.list_buckets().send().await {
            Ok(r) => r
                .buckets()
                .iter()
                .filter_map(|b| b.name().map(|n| (n.to_string(), b.bucket_region().map(String::from))))
                .collect(),
            Err(e) if !p.bucket.trim().is_empty() => {
                let bucket = p.bucket.trim();
                let region = if is_aws(p) { detect_bucket_region(p, bucket).await } else { None };
                let check = region.as_deref().map(|r| make_client_in(p, r)).unwrap_or(self.base.clone());
                check.head_bucket().bucket(bucket).send().await.map_err(|_| err(e))?;
                vec![(bucket.to_string(), region)]
            }
            Err(e) => return Err(err(e)),
        };
        let mut regions = self.regions.lock().unwrap();
        Ok(out
            .into_iter()
            .map(|(name, region)| {
                if let Some(r) = &region {
                    regions.insert(name.clone(), r.clone()); // saves a HEAD per bucket on AWS
                }
                BucketInfo { name, region }
            })
            .collect())
    }

    /// One folder level: sub-folders (common prefixes) and files directly under `prefix`.
    pub async fn list(&self, bucket: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let c = self.client_for(bucket).await;
        let mut out = vec![];
        let mut pages = c.list_objects_v2().bucket(bucket).prefix(prefix).delimiter("/").into_paginator().send();
        while let Some(page) = pages.next().await {
            let page = page.map_err(err)?;
            for p in page.common_prefixes() {
                if let Some(k) = p.prefix() {
                    out.push(Entry {
                        key: k.into(),
                        name: file_name(k),
                        size: 0,
                        modified: None,
                        dir: true,
                        hash: None,
                    });
                }
            }
            for o in page.contents() {
                let Some(k) = o.key() else { continue };
                if k == prefix {
                    continue; // the folder's own marker object
                }
                out.push(Entry {
                    key: k.into(),
                    name: file_name(k),
                    size: o.size().unwrap_or(0).max(0) as u64,
                    modified: o.last_modified().and_then(|d| d.to_millis().ok()),
                    dir: false,
                    hash: o.e_tag().map(|t| format!("etag:{}", t.trim_matches('"'))),
                });
            }
        }
        Ok(out)
    }

    /// Every file under `prefix` (recursive, folder markers left out), with its ETag.
    pub async fn list_files(&self, bucket: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let c = self.client_for(bucket).await;
        let mut out = vec![];
        let mut pages = c.list_objects_v2().bucket(bucket).prefix(prefix).into_paginator().send();
        while let Some(page) = pages.next().await {
            let page = page.map_err(err)?;
            for o in page.contents() {
                let Some(k) = o.key() else { continue };
                if k.ends_with('/') {
                    continue;
                }
                out.push(Entry {
                    key: k.into(),
                    name: file_name(k),
                    size: o.size().unwrap_or(0).max(0) as u64,
                    modified: o.last_modified().and_then(|d| d.to_millis().ok()),
                    dir: false,
                    hash: o.e_tag().map(|t| format!("etag:{}", t.trim_matches('"'))),
                });
            }
        }
        Ok(out)
    }

    /// Every object under `prefix` (recursive), as `(key, size)`.
    pub async fn list_all(&self, bucket: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        let c = self.client_for(bucket).await;
        let mut out = vec![];
        let mut pages = c.list_objects_v2().bucket(bucket).prefix(prefix).into_paginator().send();
        while let Some(page) = pages.next().await {
            let page = page.map_err(err)?;
            for o in page.contents() {
                if let Some(k) = o.key() {
                    out.push((k.to_string(), o.size().unwrap_or(0).max(0) as u64));
                }
            }
        }
        Ok(out)
    }

    /// Presigned GET URL. `inline` forces a browser-friendly Content-Type/Disposition so the
    /// preview can render objects that were uploaded as `application/octet-stream`.
    pub async fn link(&self, bucket: &str, key: &str, secs: u64, inline: bool) -> Result<String, String> {
        let c = self.client_for(bucket).await;
        let mut req = c.get_object().bucket(bucket).key(key);
        if inline {
            let mime = mime_guess::from_path(key).first_or_octet_stream().to_string();
            let mime = if mime.starts_with("text/") {
                format!("{mime}; charset=utf-8")
            } else if is_text_like(key) {
                "text/plain; charset=utf-8".to_string()
            } else {
                mime
            };
            // no-store: the webview would otherwise keep every previewed photo and video on disk.
            req = req
                .response_content_type(mime)
                .response_content_disposition("inline")
                .response_cache_control("no-store");
        }
        let cfg = PresigningConfig::expires_in(Duration::from_secs(secs.clamp(60, 604_800))).map_err(err)?;
        Ok(req.presigned(cfg).await.map_err(err)?.uri().to_string())
    }

    /// The first `max` bytes of an object.
    pub async fn read_head(&self, bucket: &str, key: &str, max: u64) -> Result<Vec<u8>, String> {
        let c = self.client_for(bucket).await;
        let r = c
            .get_object()
            .bucket(bucket)
            .key(key)
            .range(format!("bytes=0-{}", max.max(1) - 1))
            .send()
            .await
            .map_err(err)?;
        Ok(r.body.collect().await.map_err(err)?.into_bytes().to_vec())
    }

    /// Bytes `start..=end` (open-ended: at most `cap`) and the object's total size.
    pub async fn read_range(
        &self,
        bucket: &str,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        let c = self.client_for(bucket).await;
        let last = end.unwrap_or(start + cap - 1).min(start + cap - 1);
        let r =
            c.get_object().bucket(bucket).key(key).range(format!("bytes={start}-{last}")).send().await.map_err(err)?;
        let total = r.content_range().and_then(|s| s.rsplit('/').next()).and_then(|n| n.parse().ok());
        Ok((r.body.collect().await.map_err(err)?.into_bytes().to_vec(), total))
    }

    /// Server-side copy inside this account (no bytes through this computer). S3 allows up to
    /// 5 GB per CopyObject; bigger files are copied by the caller through a relay.
    pub async fn copy_within(
        &self,
        src_bucket: &str,
        src_key: &str,
        dst_bucket: &str,
        dst_key: &str,
    ) -> Result<(), String> {
        let c = self.client_for(dst_bucket).await;
        let source = format!("{src_bucket}/{}", crate::webdav::enc_path(src_key));
        c.copy_object().bucket(dst_bucket).key(dst_key).copy_source(source).send().await.map_err(err)?;
        Ok(())
    }

    /// Copies or moves (`keep` = copy) `from` to `to` in `bucket`, server-side: one object, or a
    /// folder (`…/`) with everything in it. Objects over 5 GB go through multipart copy. Refuses to
    /// overwrite an existing target. Progress: files = objects.
    pub async fn transfer_within(
        &self,
        bucket: &str,
        from: &str,
        to: &str,
        keep: bool,
        job: Option<&Job>,
    ) -> Result<(), String> {
        let c = self.client_for(bucket).await;
        if self.exists(bucket, to).await? {
            return Err(format!("“{}” already exists there", file_name(to)));
        }
        let objs = if from.ends_with('/') {
            self.list_all(bucket, from).await? // the marker object, if any, is one of them
        } else {
            let h = c.head_object().bucket(bucket).key(from).send().await.map_err(err)?;
            vec![(from.to_string(), h.content_length().unwrap_or(0).max(0) as u64)]
        };
        if let Some(j) = job {
            j.files_total.store(objs.len() as u64, Ordering::Relaxed);
        }
        for (k, size) in &objs {
            if job.is_some_and(|j| j.cancelled()) {
                return Err("cancelled".into());
            }
            let dst = format!("{to}{}", &k[from.len()..]);
            if *size > 5 * 1024 * 1024 * 1024 {
                self.copy_large(&c, bucket, k, &dst, *size).await?;
            } else {
                self.copy_within(bucket, k, bucket, &dst).await?;
            }
            if let Some(j) = job {
                j.files_done.fetch_add(1, Ordering::Relaxed);
            }
        }
        if !keep {
            self.delete(bucket, vec![from.to_string()], None).await?;
        }
        Ok(())
    }

    /// Is there an object at `key` (or, for `…/`, anything under it)?
    async fn exists(&self, bucket: &str, key: &str) -> Result<bool, String> {
        let c = self.client_for(bucket).await;
        if key.ends_with('/') {
            let r = c.list_objects_v2().bucket(bucket).prefix(key).max_keys(1).send().await.map_err(err)?;
            return Ok(r.key_count().unwrap_or(0) > 0);
        }
        match c.head_object().bucket(bucket).key(key).send().await {
            Ok(_) => Ok(true),
            Err(e) if e.raw_response().is_some_and(|r| r.status().as_u16() == 404) => Ok(false),
            Err(e) => Err(err(e)),
        }
    }

    /// CopyObject stops at 5 GB: bigger objects are copied in parts (UploadPartCopy), server-side.
    async fn copy_large(&self, c: &Client, bucket: &str, src: &str, dst: &str, size: u64) -> Result<(), String> {
        let part = (512 * 1024 * 1024u64).max(size.div_ceil(10_000));
        // Unlike CopyObject, a multipart copy doesn't carry the source's metadata: keep its type.
        let ct = c
            .head_object()
            .bucket(bucket)
            .key(src)
            .send()
            .await
            .ok()
            .and_then(|h| h.content_type)
            .unwrap_or_else(|| crate::util::upload_mime(dst, Path::new("")));
        let up = c.create_multipart_upload().bucket(bucket).key(dst).content_type(ct).send().await.map_err(err)?;
        let id = up.upload_id().ok_or("no upload id")?.to_string();
        let source = format!("{bucket}/{}", crate::webdav::enc_path(src));
        let res: Result<Vec<CompletedPart>, String> = async {
            let mut parts = vec![];
            for (i, start) in (0..size).step_by(part as usize).enumerate() {
                let end = (start + part).min(size) - 1;
                let r = c
                    .upload_part_copy()
                    .bucket(bucket)
                    .key(dst)
                    .upload_id(&id)
                    .part_number(i as i32 + 1)
                    .copy_source(&source)
                    .copy_source_range(format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(err)?;
                let etag = r.copy_part_result().and_then(|p| p.e_tag()).unwrap_or_default().to_string();
                parts.push(CompletedPart::builder().part_number(i as i32 + 1).e_tag(etag).build());
            }
            Ok(parts)
        }
        .await;
        match res {
            Ok(parts) => {
                let done = CompletedMultipartUpload::builder().set_parts(Some(parts)).build();
                c.complete_multipart_upload()
                    .bucket(bucket)
                    .key(dst)
                    .upload_id(&id)
                    .multipart_upload(done)
                    .send()
                    .await
                    .map_err(err)?;
                Ok(())
            }
            Err(e) => {
                let _ = c.abort_multipart_upload().bucket(bucket).key(dst).upload_id(&id).send().await;
                Err(e)
            }
        }
    }

    /// Creates an empty `prefix/name/` marker object so the folder shows up while empty.
    pub async fn mkdir(&self, bucket: &str, prefix: &str, name: &str) -> Result<(), String> {
        let c = self.client_for(bucket).await;
        c.put_object()
            .bucket(bucket)
            .key(format!("{prefix}{name}/"))
            .body(ByteStream::from_static(b""))
            .send()
            .await
            .map_err(err)?;
        Ok(())
    }

    /// Deletes objects; keys ending in `/` delete the whole folder recursively. Returns the count.
    pub async fn delete(&self, bucket: &str, keys: Vec<String>, job: Option<&Job>) -> Result<usize, String> {
        let c = self.client_for(bucket).await;
        let mut all = vec![];
        for k in keys {
            if k.ends_with('/') {
                all.extend(self.list_all(bucket, &k).await?.into_iter().map(|(k, _)| k));
            }
            all.push(k); // the file itself, or the folder marker (deleting a missing key is a no-op)
        }
        all.sort();
        all.dedup();
        let n = all.len();
        if let Some(j) = job {
            j.files_total.store(n as u64, Ordering::Relaxed);
        }
        for chunk in all.chunks(1000) {
            if job.is_some_and(|j| j.cancelled()) {
                return Err("cancelled".into());
            }
            let objs: Vec<ObjectIdentifier> =
                chunk.iter().filter_map(|k| ObjectIdentifier::builder().key(k).build().ok()).collect();
            let del = Delete::builder().set_objects(Some(objs)).quiet(true).build().map_err(err)?;
            let bulk = c.delete_objects().bucket(bucket).delete(del).send().await;
            if !matches!(&bulk, Ok(r) if r.errors().is_empty()) {
                // Some S3-compatible servers reject bulk delete (or fail some keys); retry one-by-one.
                for k in chunk {
                    c.delete_object().bucket(bucket).key(k).send().await.map_err(err)?;
                }
            }
            if let Some(j) = job {
                j.files_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
        }
        Ok(n)
    }

    /// Single PUT for small files, parallel multipart for large ones (aborted on failure).
    pub async fn upload_file(&self, bucket: &str, key: &str, path: &Path, size: u64, job: &Job) -> Result<(), String> {
        let c = self.client_for(bucket).await;
        let ct = crate::util::upload_mime(key, path);
        if size <= MULTIPART_THRESHOLD {
            job.gate(size).await; // one request: pause and speed limit before it, progress after
            let body = ByteStream::from_path(path).await.map_err(err)?;
            c.put_object().bucket(bucket).key(key).content_type(ct).body(body).send().await.map_err(err)?;
            job.bytes_done.fetch_add(size, Ordering::Relaxed);
            return Ok(());
        }

        let part_size = MIN_PART_SIZE.max(size.div_ceil(MAX_PARTS));
        let parts = size.div_ceil(part_size);
        // Resumable jobs continue an unfinished upload of this same file (journaled with its size
        // and modification time), sending only the parts the server doesn't have yet.
        let journal = job.resume;
        let mut have: HashMap<i32, String> = HashMap::new();
        let found = if journal { crate::resume::multipart_find(self.account(), bucket, key, size, path) } else { None };
        let upload_id = match found {
            Some(id) => match self.parts_of(&c, bucket, key, &id).await {
                Ok(p) => {
                    have = p;
                    id
                }
                Err(_) => {
                    crate::resume::multipart_remove(&id); // gone on the server: start over
                    String::new()
                }
            },
            None => String::new(),
        };
        let upload_id = if upload_id.is_empty() {
            let created =
                c.create_multipart_upload().bucket(bucket).key(key).content_type(ct).send().await.map_err(err)?;
            let id = created.upload_id().ok_or("Server returned no upload id")?.to_string();
            if journal {
                crate::resume::multipart_put(self.account(), bucket, key, size, path, &id);
            }
            id
        } else {
            upload_id
        };
        let have = &have;

        let c = &c;
        let result: Result<(), String> = async {
            let mut done: Vec<CompletedPart> = stream::iter(0..parts)
                .map(|i| {
                    let upload_id = upload_id.clone();
                    async move {
                        if job.cancelled() {
                            return Err("cancelled".to_string());
                        }
                        let offset = i * part_size;
                        let len = part_size.min(size - offset);
                        let n = (i + 1) as i32;
                        if let Some(etag) = have.get(&n) {
                            job.bytes_done.fetch_add(len, Ordering::Relaxed); // sent before
                            return Ok(CompletedPart::builder().part_number(n).e_tag(etag).build());
                        }
                        job.gate(len).await;
                        let mut f = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
                        f.seek(SeekFrom::Start(offset)).await.map_err(|e| e.to_string())?;
                        let mut buf = vec![0u8; len as usize];
                        f.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
                        let r = c
                            .upload_part()
                            .bucket(bucket)
                            .key(key)
                            .upload_id(&upload_id)
                            .part_number(n)
                            .body(ByteStream::from(buf))
                            .send()
                            .await
                            .map_err(err)?;
                        job.bytes_done.fetch_add(len, Ordering::Relaxed);
                        Ok(CompletedPart::builder().part_number(n).set_e_tag(r.e_tag().map(String::from)).build())
                    }
                })
                .buffer_unordered(PART_CONCURRENCY)
                .try_collect()
                .await?;
            done.sort_by_key(|p| p.part_number());
            c.complete_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(done)).build())
                .send()
                .await
                .map_err(err)?;
            Ok(())
        }
        .await;

        match &result {
            Ok(()) => crate::resume::multipart_remove(&upload_id),
            // Keep the parts for a resume after network trouble; otherwise free the server space.
            Err(e) if journal && !job.cancelled() && crate::resume::transient(e) => {}
            Err(_) => {
                let _ = c.abort_multipart_upload().bucket(bucket).key(key).upload_id(&upload_id).send().await;
                crate::resume::multipart_remove(&upload_id);
            }
        }
        result
    }

    /// Parts already uploaded for `upload_id` (part number → ETag).
    async fn parts_of(
        &self,
        c: &Client,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<HashMap<i32, String>, String> {
        let mut out = HashMap::new();
        let mut marker: Option<String> = None;
        loop {
            let r = c
                .list_parts()
                .bucket(bucket)
                .key(key)
                .upload_id(upload_id)
                .set_part_number_marker(marker.clone())
                .send()
                .await
                .map_err(err)?;
            for p in r.parts() {
                if let (Some(n), Some(e)) = (p.part_number(), p.e_tag()) {
                    out.insert(n, e.to_string());
                }
            }
            match (r.is_truncated(), r.next_part_number_marker()) {
                (Some(true), Some(m)) => marker = Some(m.to_string()),
                _ => return Ok(out),
            }
        }
    }

    /// Streams one object to `path` via `path.part`, renaming on success.
    pub async fn download_file(&self, bucket: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        let c = self.client_for(bucket).await;
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let tmp = part_path(path);
        let offset = resume_offset(&tmp, job).await;
        let mut req = c.get_object().bucket(bucket).key(key);
        if offset > 0 {
            req = req.range(format!("bytes={offset}-"));
        }
        let r = match req.send().await {
            Ok(r) => r,
            // The .part already has every byte (nothing left to range over).
            Err(e) if offset > 0 && e.raw_response().is_some_and(|r| r.status().as_u16() == 416) => {
                job.bytes_done.fetch_add(offset, Ordering::Relaxed);
                return tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string());
            }
            Err(e) => return Err(err(e)),
        };
        let partial = offset > 0 && r.content_range().is_some();
        let mut body = r.body;
        let f = open_part(&tmp, partial, offset, job).await?;
        let mut w = tokio::io::BufWriter::with_capacity(1 << 20, f);
        let res: Result<(), String> = async {
            while let Some(chunk) = body.try_next().await.map_err(err)? {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_id_is_removed_and_gcs_detected() {
        assert_eq!(without_x_id("https://h/b/k?x-id=GetObject"), "https://h/b/k");
        assert_eq!(
            without_x_id("https://h/b?list-type=2&x-id=ListObjectsV2&prefix=a"),
            "https://h/b?list-type=2&prefix=a"
        );
        assert_eq!(without_x_id("https://h/?x-idx=1"), "https://h/?x-idx=1");
        assert!(is_gcs("https://storage.googleapis.com"));
        assert!(is_gcs("https://storage.googleapis.com:443/"));
        assert!(!is_gcs("https://minio.example.com"));
        assert!(!is_gcs("https://storage.googleapis.com.evil.example"));
    }

    /// The project header goes on ListBuckets (service root) only.
    #[tokio::test]
    async fn gcs_list_buckets_carries_the_project() {
        let p = Profile {
            endpoint: "https://storage.googleapis.com".into(),
            access_key: "GOOG1EXAMPLE".into(),
            secret_key: "secret".into(),
            project: "my-project-123".into(),
            ..Default::default()
        };
        let c = make_client_in(&p, "auto");
        let cfg = PresigningConfig::expires_in(Duration::from_secs(60)).unwrap();
        let obj = c.get_object().bucket("b").key("k").presigned(cfg).await.unwrap();
        assert!(obj.headers().all(|(k, _)| k != "x-amz-project-id"), "not on object requests");
        assert!(is_service_root("https://storage.googleapis.com/"));
        assert!(is_service_root("https://storage.googleapis.com/?max-buckets=10"));
        assert!(is_service_root("https://storage.googleapis.com"));
        assert!(!is_service_root("https://storage.googleapis.com/b/k"));
        assert!(!is_service_root("https://storage.googleapis.com/b?list-type=2"));
        assert!(is_bare_bucket("https://storage.googleapis.com/new-bucket"));
        assert!(!is_bare_bucket("https://storage.googleapis.com/b/k"));
        assert!(!is_bare_bucket("https://storage.googleapis.com/b?uploads"));
        assert!(!is_bare_bucket("https://storage.googleapis.com/"));
    }

    /// Presigning runs the same interceptors as real requests, so this checks the client offline.
    #[tokio::test]
    async fn gcs_requests_carry_no_x_id() {
        let link = |endpoint: &str| {
            let c = make_client_in(
                &Profile {
                    endpoint: endpoint.into(),
                    access_key: "GOOG1EXAMPLE".into(),
                    secret_key: "secret".into(),
                    path_style: true,
                    ..Default::default()
                },
                "auto",
            );
            async move {
                let cfg = PresigningConfig::expires_in(Duration::from_secs(60)).unwrap();
                c.get_object().bucket("b").key("a.pdf").presigned(cfg).await.unwrap().uri().to_string()
            }
        };
        let gcs = link("https://storage.googleapis.com").await;
        assert!(gcs.starts_with("https://storage.googleapis.com/b/a.pdf?"), "{gcs}");
        assert!(!gcs.contains("x-id"), "{gcs}");
        assert!(link("https://minio.example.com").await.contains("x-id=GetObject"), "other servers unchanged");
    }
}
