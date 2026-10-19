//! Grid thumbnails (experimental "grid"): small JPEGs (PNG when the image has transparency) made
//! from the file itself, so the grid shows 20 KB pictures instead of decoding whole photos, and
//! scrolling back doesn't download anything again. Served by `proxy.rs` as
//! `nxr://localhost/~thumb/<account | ~local>/<container>/<key>?<size>-<modified>`.
//!
//! Two least-recently-used caches: memory ([`MEM_CAP`]) and disk ([`DISK_CAP`], in the OS cache
//! folder under `thumbs/`, which the OS may also clear). The key includes the file's size and
//! modified time, so an edited file gets a new thumbnail and the old one ages out. Videos keep
//! their live frame in the grid (decoding video would need a codec library).
//!
//! The webview's own disk cache is kept small too: previews are sent `no-store` (presigned links
//! and `nxr://`), and [`web_cache_plugin`] deletes what WebKit kept anyway when it's over
//! [`WEB_CACHE_MAX`] at launch. Together that's at most ~70 MB on disk.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Longest side in pixels: a grid tile is ~120 CSS px, so 256 stays sharp on Retina screens.
pub const SIZE: u32 = 256;
const QUALITY: u8 = 80;
const MEM_CAP: usize = 16 << 20;
const DISK_CAP: u64 = 48 << 20;
/// Trimming goes below the cap, so not every new thumbnail triggers a trim.
const DISK_LOW: u64 = 40 << 20;
/// WebKit's disk caches (macOS) bigger than this are deleted at launch.
const WEB_CACHE_MAX: u64 = 16 << 20;
/// Biggest original read: remote (it's downloaded once per thumbnail) and local.
pub const REMOTE_MAX: u64 = 25 << 20;
const LOCAL_MAX: u64 = 64 << 20;
/// Thumbnails made at once (download + decode); the rest wait their turn.
const WORKERS: usize = 4;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static MEM: LazyLock<Mutex<Lru>> = LazyLock::new(|| Mutex::new(Lru::new(MEM_CAP)));
/// Bytes in the disk cache; `None` until first measured.
static DISK_USED: Mutex<Option<u64>> = Mutex::new(None);
static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(WORKERS);

/// Trims the webview's disk caches at launch. Plugins are set up before the window (and its
/// webview) is created; registered right after single-instance, so a second launch, which hands
/// over and exits there, never touches the running app's cache.
pub fn web_cache_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("web-cache")
        .setup(|app, _| {
            for dir in web_cache_dirs(&app.config().identifier) {
                if dir_size(&dir) > WEB_CACHE_MAX {
                    let _ = std::fs::remove_dir_all(&dir);
                }
            }
            Ok(())
        })
        .build()
}

/// Where WebKit keeps HTTP responses and media for this app. WebView2 (Windows) and WebKitGTK
/// size their caches themselves.
fn web_cache_dirs(identifier: &str) -> Vec<PathBuf> {
    let mut v = vec![];
    if cfg!(target_os = "macos") {
        if let Some(c) = dirs::cache_dir() {
            v.push(c.join(identifier).join("WebKit").join("NetworkCache"));
        }
        v.push(std::env::temp_dir().join(identifier).join("WebKit").join("MediaCache"));
    }
    v
}

fn dir_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

pub fn init(app: &AppHandle) {
    if let Ok(d) = app.path().app_cache_dir() {
        let _ = DIR.set(d.join("thumbs"));
    }
}

/// The thumbnail of a file (`account` = [`crate::proxy::LOCAL`] for this computer).
pub async fn get(
    app: &AppHandle,
    account: &str,
    container: &str,
    key: &str,
    stamp: &str,
) -> Result<Arc<Vec<u8>>, String> {
    let id = cache_key(account, container, key, stamp);
    if let Some(b) = MEM.lock().unwrap().get(&id) {
        return Ok(b);
    }
    let file = DIR.get().map(|d| d.join(&id));
    if let Some(b) = from_disk(file.as_deref()).await {
        return Ok(MEM.lock().unwrap().put(id, b));
    }
    let _turn = GATE.acquire().await.map_err(|e| e.to_string())?;
    // Made by another request while this one waited?
    if let Some(b) = MEM.lock().unwrap().get(&id) {
        return Ok(b);
    }
    let src = if account == crate::proxy::LOCAL {
        let (b, total) = crate::proxy::read_local(key, 0, None, LOCAL_MAX).await?;
        too_big(total, LOCAL_MAX)?;
        b
    } else {
        let remote = app.state::<AppState>().remote(Some(account))?;
        let (b, total) = remote.read_range(container, key, 0, None, REMOTE_MAX).await?;
        too_big(total, REMOTE_MAX)?;
        b
    };
    let thumb = tokio::task::spawn_blocking(move || {
        let t = make(&src)?;
        if let Some(f) = file {
            save(&f, &t);
        }
        Ok::<_, String>(t)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(MEM.lock().unwrap().put(id, Arc::new(thumb)))
}

fn too_big(total: Option<u64>, max: u64) -> Result<(), String> {
    match total {
        Some(t) if t > max => Err("Too big for a thumbnail".into()),
        _ => Ok(()),
    }
}

/// `image/png` or `image/jpeg`, from the thumbnail's first bytes.
pub fn mime(b: &[u8]) -> &'static str {
    if b.starts_with(b"\x89PNG") { "image/png" } else { "image/jpeg" }
}

fn cache_key(account: &str, container: &str, key: &str, stamp: &str) -> String {
    let h = Sha256::digest(format!("v1\0{SIZE}\0{account}\0{container}\0{key}\0{stamp}"));
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes, turns the way the camera says (EXIF), shrinks and encodes the thumbnail.
fn make(src: &[u8]) -> Result<Vec<u8>, String> {
    use image::{DynamicImage, ImageDecoder, ImageReader};
    let e = |e: image::ImageError| e.to_string();
    let mut dec = ImageReader::new(Cursor::new(src))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(e)?;
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(dec).map_err(e)?;
    img.apply_orientation(orientation);
    let t = img.thumbnail(SIZE, SIZE);
    let mut out = vec![];
    if t.color().has_alpha() {
        t.into_rgba8().write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).map_err(e)?;
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY)
            .encode_image(&t.into_rgb8())
            .map_err(e)?;
    }
    Ok(out)
}

async fn from_disk(file: Option<&Path>) -> Option<Arc<Vec<u8>>> {
    let file = file?;
    let b = tokio::fs::read(file).await.ok()?;
    // Recently used = recently modified: that's what trimming goes by.
    let _ = std::fs::File::options().write(true).open(file).and_then(|f| f.set_modified(SystemTime::now()));
    Some(Arc::new(b))
}

fn save(file: &Path, b: &[u8]) {
    let Some(dir) = file.parent() else { return };
    let tmp = file.with_extension("part");
    if std::fs::create_dir_all(dir).is_err() || std::fs::write(&tmp, b).is_err() || std::fs::rename(&tmp, file).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    let mut used = DISK_USED.lock().unwrap();
    let u = used.get_or_insert_with(|| trim(dir, u64::MAX));
    *u += b.len() as u64;
    if *u > DISK_CAP {
        *u = trim(dir, DISK_LOW);
    }
}

/// Deletes the least recently used thumbnails until the folder holds at most `target` bytes.
/// Returns what's left. Only this module writes in the folder.
fn trim(dir: &Path, target: u64) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
    total
}

/// Least-recently-used map of thumbnails, bounded by total bytes.
struct Lru {
    map: HashMap<String, (Arc<Vec<u8>>, u64)>,
    tick: u64,
    bytes: usize,
    cap: usize,
}

impl Lru {
    fn new(cap: usize) -> Self {
        Self { map: HashMap::new(), tick: 0, bytes: 0, cap }
    }

    fn get(&mut self, k: &str) -> Option<Arc<Vec<u8>>> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.1 = t;
            e.0.clone()
        })
    }

    /// Stores `v` (evicting the least recently used) and hands it back.
    fn put(&mut self, k: String, v: Arc<Vec<u8>>) -> Arc<Vec<u8>> {
        if v.len() > self.cap {
            return v;
        }
        self.tick += 1;
        self.bytes += v.len();
        if let Some((old, _)) = self.map.insert(k, (v.clone(), self.tick)) {
            self.bytes -= old.len();
        }
        while self.bytes > self.cap {
            // A linear scan: the map holds a few hundred thumbnails.
            let Some(oldest) = self.map.iter().min_by_key(|(_, e)| e.1).map(|(k, _)| k.clone()) else { break };
            if let Some((gone, _)) = self.map.remove(&oldest) {
                self.bytes -= gone.len();
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_keeps_recent_within_cap() {
        let mut l = Lru::new(30);
        let b = |n: usize| Arc::new(vec![0u8; n]);
        l.put("a".into(), b(10));
        l.put("b".into(), b(10));
        l.put("c".into(), b(10));
        assert!(l.get("a").is_some()); // a is now the most recent
        l.put("d".into(), b(10)); // evicts b
        assert!(l.get("b").is_none());
        assert!(l.get("a").is_some() && l.get("c").is_some() && l.get("d").is_some());
        assert_eq!(l.bytes, 30);
        l.put("a".into(), b(5)); // replacing counts the new size only
        assert_eq!(l.bytes, 25);
        l.put("huge".into(), b(100)); // bigger than the cap: not kept
        assert!(l.get("huge").is_none());
    }

    #[test]
    fn thumbnails_are_small_and_upright() {
        // 800×400 JPEG tagged "rotate 90°" (EXIF orientation 6) → 128×256 after turning.
        let img = image::RgbImage::from_fn(800, 400, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90]));
        let mut jpg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(&img).unwrap();
        let t = make(&jpg).unwrap();
        assert_eq!(mime(&t), "image/jpeg");
        let back = image::load_from_memory(&t).unwrap();
        assert_eq!((back.width(), back.height()), (256, 128));
        let rotated = with_exif_orientation(&jpg, 6);
        let back = image::load_from_memory(&make(&rotated).unwrap()).unwrap();
        assert_eq!((back.width(), back.height()), (128, 256));
        // Transparency stays: PNG.
        let rgba = image::RgbaImage::from_pixel(300, 300, image::Rgba([0, 0, 0, 0]));
        let mut png = vec![];
        rgba.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(mime(&make(&png).unwrap()), "image/png");
        assert!(make(b"not an image").is_err());
    }

    /// Inserts a minimal EXIF APP1 segment with an orientation tag after the JPEG's SOI marker.
    fn with_exif_orientation(jpg: &[u8], o: u16) -> Vec<u8> {
        let mut tiff = b"MM\0\x2a\0\0\0\x08\0\x01".to_vec(); // big-endian, IFD at 8, 1 entry
        tiff.extend([0x01, 0x12, 0, 3, 0, 0, 0, 1]); // tag 0x0112, SHORT, count 1
        tiff.extend(o.to_be_bytes());
        tiff.extend([0, 0, 0, 0, 0, 0]); // value padding + next IFD = 0
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut out = jpg[..2].to_vec();
        out.extend([0xFF, 0xE1]);
        out.extend(((app1.len() + 2) as u16).to_be_bytes());
        out.extend(app1);
        out.extend(&jpg[2..]);
        out
    }

    #[test]
    fn trim_drops_oldest_first() {
        let dir = std::env::temp_dir().join(format!("noexplorer-ut-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let t0 = SystemTime::now() - std::time::Duration::from_secs(100);
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            let p = dir.join(name);
            std::fs::write(&p, vec![0u8; 10]).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(t0 + std::time::Duration::from_secs(i as u64 * 10)).unwrap();
        }
        assert_eq!(trim(&dir, u64::MAX), 30);
        assert_eq!(trim(&dir, 20), 20);
        assert!(!dir.join("old").exists() && dir.join("mid").exists() && dir.join("new").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
