//! Move to Trash for the local pane (ADR-023). The app still never deletes local files for good
//! (ADR-006): everything goes where the OS file manager can put it back.
//!
//! - macOS: `NSFileManager trashItemAtURL` (Finder's "Put Back" works).
//! - Windows: `SHFileOperationW` with `FOF_ALLOWUNDO`; if an item can't be recycled (too big, a
//!   drive without a Recycle Bin) Windows asks before deleting it (`FOF_WANTNUKEWARNING`).
//! - Linux & other unix: the freedesktop.org Trash spec (home trash, or `$topdir/.Trash-$uid` on
//!   other filesystems), so GNOME Files / Dolphin show and restore the items.

use std::path::{Path, PathBuf};

/// Moves local files/folders to the Trash. Returns how many were moved.
#[tauri::command]
pub async fn trash_local(paths: Vec<String>) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let mut n = 0;
        for p in &paths {
            let path = PathBuf::from(p);
            guard(&path)?;
            trash(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            n += 1;
        }
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Refuses paths that are never a sensible thing to trash from a file list.
fn guard(p: &Path) -> Result<(), String> {
    if !p.is_absolute() || p.parent().is_none() {
        return Err(format!("{}: not a file or folder that can be moved to the Trash", p.display()));
    }
    if let Some(home) = dirs::home_dir()
        && home.starts_with(p)
    {
        return Err(format!(
            "{}: the home folder (or a folder containing it) can't be moved to the Trash",
            p.display()
        ));
    }
    std::fs::symlink_metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn trash(p: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|e| e.localizedDescription().to_string())
}

#[cfg(windows)]
fn trash(p: &Path) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    // shellapi.h; natural alignment on 64-bit Windows.
    #[repr(C)]
    struct ShFileOpStructW {
        hwnd: *mut c_void,
        w_func: u32,
        p_from: *const u16,
        p_to: *const u16,
        f_flags: u16,
        f_any_operations_aborted: i32,
        h_name_mappings: *mut c_void,
        lpsz_progress_title: *const u16,
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHFileOperationW(op: *mut ShFileOpStructW) -> i32;
    }
    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_WANTNUKEWARNING: u16 = 0x4000;

    // Double-NUL-terminated list of one path.
    let mut from: Vec<u16> = p.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = ShFileOpStructW {
        hwnd: std::ptr::null_mut(),
        w_func: FO_DELETE,
        p_from: from.as_ptr(),
        p_to: std::ptr::null(),
        f_flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_SILENT,
        f_any_operations_aborted: 0,
        h_name_mappings: std::ptr::null_mut(),
        lpsz_progress_title: std::ptr::null(),
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        return Err(format!("Windows couldn't move it to the Recycle Bin (error {rc:#x})"));
    }
    if op.f_any_operations_aborted != 0 {
        return Err("Cancelled".into());
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn trash(p: &Path) -> Result<(), String> {
    freedesktop::trash(p).map_err(|e| e.to_string())
}

/// The freedesktop.org Trash specification 1.0.
#[cfg(all(unix, not(target_os = "macos")))]
mod freedesktop {
    use std::io::{self, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    pub fn trash(p: &Path) -> io::Result<()> {
        let p = std::path::absolute(p)?;
        let dev = std::fs::symlink_metadata(&p)?.dev();
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|d| d.is_absolute())
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .ok_or_else(|| io::Error::other("no home folder"))?;
        std::fs::create_dir_all(&data_home)?;
        let (trash, recorded) = if std::fs::metadata(&data_home)?.dev() == dev {
            (data_home.join("Trash"), p.clone())
        } else {
            let top = topdir(&p, dev);
            (top_trash(&top)?, p.strip_prefix(&top).map(Path::to_path_buf).unwrap_or(p.clone()))
        };
        let files = trash.join("files");
        let info = trash.join("info");
        for d in [&trash, &files, &info] {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d)?;
        }
        let name = p.file_name().ok_or_else(|| io::Error::other("no file name"))?.to_string_lossy().into_owned();
        // Reserve a unique name by creating its .trashinfo exclusively (the spec's locking rule).
        for i in 1u32.. {
            let candidate = if i == 1 { name.clone() } else { format!("{name}.{i}") };
            let info_path = info.join(format!("{candidate}.trashinfo"));
            let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&info_path) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            };
            write!(f, "[Trash Info]\nPath={}\nDeletionDate={}\n", url_encode(&recorded), now_local())?;
            drop(f);
            if files.join(&candidate).symlink_metadata().is_ok() {
                let _ = std::fs::remove_file(&info_path);
                continue;
            }
            return std::fs::rename(&p, files.join(&candidate)).inspect_err(|_| {
                let _ = std::fs::remove_file(&info_path);
            });
        }
        unreachable!()
    }

    /// Mount point of the filesystem holding `p`.
    fn topdir(p: &Path, dev: u64) -> PathBuf {
        let mut top = p.to_path_buf();
        while let Some(parent) = top.parent() {
            if std::fs::metadata(parent).ok().map(|m| m.dev()) != Some(dev) {
                break;
            }
            top = parent.to_path_buf();
        }
        top
    }

    /// `$topdir/.Trash/$uid` when the admin made a valid shared `.Trash` (sticky, not a link),
    /// else `$topdir/.Trash-$uid`.
    fn top_trash(top: &Path) -> io::Result<PathBuf> {
        let uid = unsafe { libc::getuid() };
        let shared = top.join(".Trash");
        if let Ok(m) = std::fs::symlink_metadata(&shared)
            && m.is_dir()
            && m.permissions().mode() & 0o1000 != 0
        {
            return Ok(shared.join(uid.to_string()));
        }
        Ok(top.join(format!(".Trash-{uid}")))
    }

    fn url_encode(p: &Path) -> String {
        use std::os::unix::ffi::OsStrExt;
        let mut out = String::new();
        for &b in p.as_os_str().as_bytes() {
            if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    /// `YYYY-MM-DDThh:mm:ss` in local time, as the spec asks.
    fn now_local() -> String {
        unsafe {
            let t = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&t, &mut tm);
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min,
                tm.tm_sec
            )
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn trashes_into_xdg_data_home_with_info() {
            let base = std::env::temp_dir().join(format!("nx-trash-{}", uuid::Uuid::new_v4()));
            let data = base.join("data");
            let f = base.join("a b.txt");
            std::fs::create_dir_all(&data).unwrap();
            std::fs::write(&f, "x").unwrap();
            unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
            trash(&f).unwrap();
            std::fs::write(&f, "y").unwrap();
            trash(&f).unwrap();
            assert!(!f.exists());
            assert!(data.join("Trash/files/a b.txt").exists() && data.join("Trash/files/a b.txt.2").exists());
            let info = std::fs::read_to_string(data.join("Trash/info/a b.txt.trashinfo")).unwrap();
            assert!(info.starts_with("[Trash Info]\nPath=/") && info.contains("a%20b.txt\nDeletionDate=20"));
            std::fs::remove_dir_all(&base).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_refuses_roots_home_and_relative_paths() {
        assert!(guard(Path::new("/")).is_err());
        assert!(guard(Path::new("relative/file")).is_err());
        if let Some(home) = dirs::home_dir() {
            assert!(guard(&home).is_err());
            assert!(guard(home.parent().unwrap()).is_err());
        }
        assert!(guard(Path::new("/definitely/not/here.txt")).is_err());
    }

    /// Moves a real temp file to the user's Trash: `cargo test trash_roundtrip -- --ignored`.
    #[test]
    #[ignore]
    fn trash_roundtrip() {
        let f = std::env::temp_dir().join(format!("noexplorer-trash-test-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&f, "noexplorer").unwrap();
        guard(&f).unwrap();
        trash(&f).unwrap();
        assert!(!f.exists());
    }
}
