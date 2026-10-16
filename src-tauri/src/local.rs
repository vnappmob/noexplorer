//! Local filesystem commands for the left pane. Intentionally read-mostly: the app never
//! deletes or overwrites local files (see ADR-006).

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncReadExt;

use crate::provider::Entry;
use crate::remote::SizeInfo;

#[derive(Serialize)]
pub struct Crumb {
    name: String,
    path: String,
}

/// Mirrors `LocalListing` in `src/types.ts`. Entries reuse [`Entry`] with `key` = absolute path.
#[derive(Serialize)]
pub struct LocalListing {
    path: String,
    parent: Option<String>,
    crumbs: Vec<Crumb>,
    roots: Vec<Crumb>,
    entries: Vec<Entry>,
}

fn crumb_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string())
}

/// Home, common folders, filesystem roots and mounted volumes for the "Places" picker.
fn local_roots() -> Vec<Crumb> {
    let mut out: Vec<Crumb> = vec![];
    let mut push = |p: PathBuf, name: String| {
        let path = p.to_string_lossy().to_string();
        if p.is_dir() && !out.iter().any(|c| c.path == path) {
            out.push(Crumb { name, path });
        }
    };
    if let Some(h) = dirs::home_dir() {
        push(h, "🏠 Home".into());
    }
    for (d, n) in
        [(dirs::desktop_dir(), "Desktop"), (dirs::document_dir(), "Documents"), (dirs::download_dir(), "Downloads")]
    {
        if let Some(d) = d {
            push(d, n.into());
        }
    }
    #[cfg(windows)]
    for l in b'A'..=b'Z' {
        push(PathBuf::from(format!("{}:\\", l as char)), format!("💽 {}:", l as char));
    }
    #[cfg(not(windows))]
    {
        push(PathBuf::from("/"), "💽 /".into());
        for base in ["/Volumes", "/media", "/mnt", "/run/media"] {
            let mut found: Vec<PathBuf> =
                std::fs::read_dir(base).into_iter().flatten().flatten().map(|e| e.path()).collect();
            // Linux mounts live one level deeper: /media/<user>/<volume>.
            if base != "/Volumes" {
                let nested: Vec<PathBuf> = found
                    .iter()
                    .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
                    .collect();
                found.extend(nested);
            }
            for d in found {
                let n = crumb_name(&d);
                push(d, format!("💽 {n}"));
            }
        }
    }
    out
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.') || name == "desktop.ini" || name == "Thumbs.db"
}

/// Lists one local folder (home when `path` is empty).
#[tauri::command]
pub async fn list_local(
    idx: tauri::State<'_, crate::sizes::SizeIndex>,
    path: Option<String>,
    hidden: bool,
) -> Result<LocalListing, String> {
    let listing =
        tokio::task::spawn_blocking(move || read_listing(path, hidden)).await.map_err(|e| e.to_string())??;
    // Opening a folder grants access to it (macOS privacy): automatic size scans may enter it now.
    idx.opened(Path::new(&listing.path));
    Ok(listing)
}

fn read_listing(path: Option<String>, hidden: bool) -> Result<LocalListing, String> {
    let p = match path.filter(|p| !p.trim().is_empty()) {
        Some(p) => PathBuf::from(p),
        None => dirs::home_dir().ok_or("Home folder not found")?,
    };
    let rd = std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut entries = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !hidden && is_hidden(&name) {
            continue;
        }
        let path = e.path();
        // Follow symlinks; fall back to the link itself if it's broken.
        let Ok(md) = std::fs::metadata(&path).or_else(|_| e.metadata()) else { continue };
        entries.push(Entry {
            key: path.to_string_lossy().to_string(),
            name,
            size: if md.is_dir() { 0 } else { md.len() },
            modified: md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64),
            dir: md.is_dir(),
            hash: None,
        });
    }
    let mut crumbs: Vec<Crumb> =
        p.ancestors().map(|a| Crumb { name: crumb_name(a), path: a.to_string_lossy().to_string() }).collect();
    crumbs.reverse();
    Ok(LocalListing {
        path: p.to_string_lossy().to_string(),
        parent: p.parent().map(|x| x.to_string_lossy().to_string()),
        crumbs,
        roots: local_roots(),
        entries,
    })
}

/// Recursive size of a local folder (symlinks are not followed, so loops can't happen).
#[tauri::command]
pub async fn local_size(path: String) -> Result<SizeInfo, String> {
    tokio::task::spawn_blocking(move || {
        let (mut bytes, mut files) = (0, 0);
        for e in walkdir::WalkDir::new(&path).into_iter().filter_map(Result::ok) {
            if e.file_type().is_file() {
                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                files += 1;
            }
        }
        SizeInfo { bytes, files }
    })
    .await
    .map_err(|e| e.to_string())
}

/// The disk holding a folder, for the storage bar under the local pane. Mirrors `VolumeInfo` in
/// `src/types.ts`.
#[derive(Serialize, Debug)]
pub struct VolumeInfo {
    /// "Macintosh HD", "Local Disk (C:)", "/home"…
    name: String,
    total: u64,
    /// What the OS calls available (macOS: including purgeable space, as System Settings shows).
    available: u64,
}

#[tauri::command]
pub async fn volume_info(path: String) -> Result<VolumeInfo, String> {
    tokio::task::spawn_blocking(move || volume(Path::new(&path)).map_err(|e| format!("{path}: {e}")))
        .await
        .map_err(|e| e.to_string())?
}

/// macOS: the same numbers as System Settings → Storage (NSURL volume resource values).
#[cfg(target_os = "macos")]
fn volume(p: &Path) -> Result<VolumeInfo, String> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{
        NSNumber, NSString, NSURL, NSURLResourceKey, NSURLVolumeAvailableCapacityForImportantUsageKey,
        NSURLVolumeAvailableCapacityKey, NSURLVolumeLocalizedNameKey, NSURLVolumeTotalCapacityKey,
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
    let get = |key: &NSURLResourceKey| -> Option<Retained<AnyObject>> {
        let mut v = None;
        // SAFETY: `v` receives whatever object type the key documents; we downcast before use.
        unsafe { url.getResourceValue_forKey_error(&mut v, key) }.ok()?;
        v
    };
    let num = |key: &NSURLResourceKey| get(key)?.downcast::<NSNumber>().ok().map(|n| n.longLongValue().max(0) as u64);
    // SAFETY: Foundation's constant keys, valid for the process lifetime.
    let (k_total, k_important, k_avail, k_name) = unsafe {
        (
            NSURLVolumeTotalCapacityKey,
            NSURLVolumeAvailableCapacityForImportantUsageKey,
            NSURLVolumeAvailableCapacityKey,
            NSURLVolumeLocalizedNameKey,
        )
    };
    let total = num(k_total).ok_or("volume size unknown")?;
    let available = num(k_important).filter(|&a| a > 0).or_else(|| num(k_avail)).unwrap_or(0);
    let name = get(k_name).and_then(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string()).unwrap_or_default();
    Ok(VolumeInfo { name, total, available })
}

#[cfg(windows)]
fn volume(p: &Path) -> Result<VolumeInfo, String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        #[allow(clippy::too_many_arguments)]
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_len: u32,
            serial: *mut u32,
            max_component: *mut u32,
            flags: *mut u32,
            fs_name: *mut u16,
            fs_name_len: u32,
        ) -> i32;
    }
    let root: PathBuf = p.ancestors().last().map(Path::to_path_buf).unwrap_or_else(|| p.to_path_buf());
    let wide = |q: &Path| q.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    if unsafe { GetDiskFreeSpaceExW(wide(p).as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut label = [0u16; 261];
    let ok = unsafe {
        GetVolumeInformationW(
            wide(&root).as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    } != 0;
    let label = if ok {
        String::from_utf16_lossy(&label[..label.iter().position(|&c| c == 0).unwrap_or(0)])
    } else {
        String::new()
    };
    let drive = root.to_string_lossy().trim_end_matches('\\').to_string();
    let name = if label.is_empty() { format!("Local Disk ({drive})") } else { format!("{label} ({drive})") };
    Ok(VolumeInfo { name, total, available: avail })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn volume(p: &Path) -> Result<VolumeInfo, String> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let block = st.f_frsize as u64;
    // Name it after its mount point ("/" → the machine's file system).
    let dev = std::fs::metadata(p).map_err(|e| e.to_string())?.dev();
    let mut mount = p.to_path_buf();
    while let Some(parent) = mount.parent() {
        if std::fs::metadata(parent).ok().map(|m| m.dev()) != Some(dev) {
            break;
        }
        mount = parent.to_path_buf();
    }
    let name = match mount.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => "File System".into(),
    };
    Ok(VolumeInfo { name, total: st.f_blocks as u64 * block, available: st.f_bavail as u64 * block })
}

/// First `max` bytes of a local file as (lossy) UTF-8, for the text preview.
#[tauri::command]
pub async fn read_local_text(path: String, max: u64) -> Result<String, String> {
    let f = tokio::fs::File::open(&path).await.map_err(|e| e.to_string())?;
    let mut buf = vec![];
    f.take(max).read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[tauri::command]
pub async fn mkdir_local(path: String, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err("Invalid folder name".into());
    }
    tokio::fs::create_dir(PathBuf::from(path).join(name)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_local(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

/// Shows a file/folder in Finder / Explorer / the Linux file manager.
#[tauri::command]
pub fn reveal(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lists_folder_and_hides_dotfiles() {
        let dir = std::env::temp_dir().join(format!("noexplorer-ut-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "abc").unwrap();
        std::fs::write(dir.join(".secret"), "x").unwrap();
        let path = Some(dir.to_string_lossy().to_string());

        let l = read_listing(path.clone(), false).unwrap();
        let mut names: Vec<_> = l.entries.iter().map(|e| (e.name.as_str(), e.dir, e.size)).collect();
        names.sort();
        assert_eq!(names, [("a.txt", false, 3), ("sub", true, 0)]);
        assert_eq!(l.crumbs.last().unwrap().path, l.path);

        let l = read_listing(path, true).unwrap();
        assert_eq!(l.entries.len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn volume_of_the_temp_dir_is_sane() {
        let v = volume(&std::env::temp_dir()).unwrap();
        assert!(v.total > 0 && v.available <= v.total, "{v:?}");
    }

    #[tokio::test]
    async fn local_size_sums_nested_files() {
        let dir = std::env::temp_dir().join(format!("noexplorer-ut-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("x.bin"), [0u8; 10]).unwrap();
        std::fs::write(dir.join("a/b/y.bin"), [0u8; 5]).unwrap();
        let s = local_size(dir.to_string_lossy().into()).await.unwrap();
        assert_eq!((s.bytes, s.files), (15, 2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn mkdir_local_rejects_bad_names() {
        for bad in ["", "..", "a/b", "a\\b"] {
            assert!(mkdir_local("/tmp".into(), bad.into()).await.is_err(), "{bad:?}");
        }
    }
}
