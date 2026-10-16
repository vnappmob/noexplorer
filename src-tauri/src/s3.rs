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
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{
    BehaviorVersion, Builder, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart, Delete, ObjectIdentifier};
use futures::{StreamExt, TryStreamExt, stream};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::config::{Profile, effective_region, is_aws, normalize_endpoint};
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry};
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
            req = req.response_content_type(mime).response_content_disposition("inline");
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
    pub async fn delete(&self, bucket: &str, keys: Vec<String>) -> Result<usize, String> {
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
        for chunk in all.chunks(1000) {
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
        }
        Ok(n)
    }

    /// Single PUT for small files, parallel multipart for large ones (aborted on failure).
    pub async fn upload_file(&self, bucket: &str, key: &str, path: &Path, size: u64, job: &Job) -> Result<(), String> {
        let c = self.client_for(bucket).await;
        let ct = mime_guess::from_path(path).first_or_octet_stream().to_string();
        if size <= MULTIPART_THRESHOLD {
            let body = ByteStream::from_path(path).await.map_err(err)?;
            c.put_object().bucket(bucket).key(key).content_type(ct).body(body).send().await.map_err(err)?;
            job.bytes_done.fetch_add(size, Ordering::Relaxed);
            return Ok(());
        }

        let part_size = MIN_PART_SIZE.max(size.div_ceil(MAX_PARTS));
        let parts = size.div_ceil(part_size);
        let created = c.create_multipart_upload().bucket(bucket).key(key).content_type(ct).send().await.map_err(err)?;
        let upload_id = created.upload_id().ok_or("Server returned no upload id")?.to_string();

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
                        let mut f = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
                        f.seek(SeekFrom::Start(offset)).await.map_err(|e| e.to_string())?;
                        let mut buf = vec![0u8; len as usize];
                        f.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
                        let n = (i + 1) as i32;
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

        if result.is_err() {
            let _ = c.abort_multipart_upload().bucket(bucket).key(key).upload_id(&upload_id).send().await;
        }
        result
    }

    /// Streams one object to `path` via `path.part`, renaming on success.
    pub async fn download_file(&self, bucket: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        let c = self.client_for(bucket).await;
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".part");
        let tmp = PathBuf::from(tmp);
        let r = c.get_object().bucket(bucket).key(key).send().await.map_err(err)?;
        let mut body = r.body;
        let f = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
        let mut w = tokio::io::BufWriter::with_capacity(1 << 20, f);
        let res: Result<(), String> = async {
            while let Some(chunk) = body.try_next().await.map_err(err)? {
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
