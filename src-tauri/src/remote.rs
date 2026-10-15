//! S3 browsing commands for the right pane: list, presign, read, mkdir, delete.
//!
//! Folders are virtual in S3: a "folder" is a common prefix ending in `/` (listed with
//! `delimiter("/")`), optionally backed by an empty `prefix/` marker object (what `mkdir` creates).

use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use serde::Serialize;
use tauri::State;

use crate::state::AppState;
use crate::util::{err, file_name, is_text_like};

/// One row in a pane. Mirrors `Item` in `src/types.ts` (local entries use the same shape).
#[derive(Serialize)]
pub struct Entry {
    /// Full object key; folders end with `/`.
    pub key: String,
    pub name: String,
    pub size: u64,
    /// Unix millis.
    pub modified: Option<i64>,
    pub dir: bool,
}

/// Every object under `prefix` (recursive), as `(key, size)`.
pub async fn list_all(c: &Client, bucket: &str, prefix: &str) -> Result<Vec<(String, u64)>, String> {
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

/// One folder level: sub-folders (common prefixes) and files directly under `prefix`.
#[tauri::command]
pub async fn list(state: State<'_, AppState>, bucket: String, prefix: String) -> Result<Vec<Entry>, String> {
    let c = state.client()?;
    let mut out = vec![];
    let mut pages = c.list_objects_v2().bucket(&bucket).prefix(&prefix).delimiter("/").into_paginator().send();
    while let Some(page) = pages.next().await {
        let page = page.map_err(err)?;
        for p in page.common_prefixes() {
            if let Some(k) = p.prefix() {
                out.push(Entry { key: k.into(), name: file_name(k), size: 0, modified: None, dir: true });
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
            });
        }
    }
    Ok(out)
}

/// Presigned GET URL. `inline` forces a browser-friendly Content-Type/Disposition so the
/// preview can render objects that were uploaded as `application/octet-stream`.
#[tauri::command]
pub async fn presign(
    state: State<'_, AppState>,
    bucket: String,
    key: String,
    secs: u64,
    inline: bool,
) -> Result<String, String> {
    let c = state.client()?;
    let mut req = c.get_object().bucket(&bucket).key(&key);
    if inline {
        let mime = mime_guess::from_path(&key).first_or_octet_stream().to_string();
        let mime = if mime.starts_with("text/") {
            format!("{mime}; charset=utf-8")
        } else if is_text_like(&key) {
            "text/plain; charset=utf-8".to_string()
        } else {
            mime
        };
        req = req.response_content_type(mime).response_content_disposition("inline");
    }
    let cfg = PresigningConfig::expires_in(Duration::from_secs(secs.clamp(60, 604_800))).map_err(err)?;
    Ok(req.presigned(cfg).await.map_err(err)?.uri().to_string())
}

/// First `max` bytes of an object as (lossy) UTF-8, for the text preview.
#[tauri::command]
pub async fn read_text(state: State<'_, AppState>, bucket: String, key: String, max: u64) -> Result<String, String> {
    let c = state.client()?;
    let r = c
        .get_object()
        .bucket(&bucket)
        .key(&key)
        .range(format!("bytes=0-{}", max.max(1) - 1))
        .send()
        .await
        .map_err(err)?;
    let bytes = r.body.collect().await.map_err(err)?.into_bytes();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Creates an empty `prefix/name/` marker object so the folder shows up while empty.
#[tauri::command]
pub async fn mkdir(state: State<'_, AppState>, bucket: String, prefix: String, name: String) -> Result<(), String> {
    let c = state.client()?;
    let name = name.trim().trim_matches('/');
    if name.is_empty() {
        return Err("Folder name is empty".into());
    }
    c.put_object()
        .bucket(&bucket)
        .key(format!("{prefix}{name}/"))
        .body(ByteStream::from_static(b""))
        .send()
        .await
        .map_err(err)?;
    Ok(())
}

/// Deletes objects; keys ending in `/` delete the whole folder recursively. Returns the count.
#[tauri::command]
pub async fn delete(state: State<'_, AppState>, bucket: String, keys: Vec<String>) -> Result<usize, String> {
    let c = state.client()?;
    let mut all = vec![];
    for k in keys {
        if k.ends_with('/') {
            all.extend(list_all(&c, &bucket, &k).await?.into_iter().map(|(k, _)| k));
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
        let bulk = c.delete_objects().bucket(&bucket).delete(del).send().await;
        if !matches!(&bulk, Ok(r) if r.errors().is_empty()) {
            // Some S3-compatible servers reject bulk delete (or fail some keys); retry one-by-one.
            for k in chunk {
                c.delete_object().bucket(&bucket).key(k).send().await.map_err(err)?;
            }
        }
    }
    Ok(n)
}
