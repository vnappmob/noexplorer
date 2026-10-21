//! Folders kept in sync by a cloud service: iCloud Drive, Google Drive, OneDrive, Dropbox, Box,
//! Nextcloud/ownCloud, Synology Drive, MEGA (ADR-049).
//!
//! Size scans don't enter them: on macOS most are File Provider folders, where listing a folder
//! can fetch it from the server and needs its own permission, and either way their contents live
//! in the cloud, not on this disk's budget. They're recognized two ways, both free during a scan:
//! - where they are: `~/Library/CloudStorage/*`, iCloud's `~/Library/Mobile Documents`, and on
//!   Windows the OneDrive / iCloudDrive folders in the home folder;
//! - a marker file the sync client keeps in the folder's top level (read with the listing anyway).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The service whose sync client keeps a file named `name` at the top of its folder.
pub fn marker(name: &[u8]) -> Option<&'static str> {
    if name.first() != Some(&b'.') {
        return None;
    }
    let n = String::from_utf8_lossy(name);
    Some(match &*n {
        // Nextcloud and ownCloud desktop clients: the sync journal.
        _ if (n.starts_with(".sync_") || n.starts_with("._sync_")) && n.ends_with(".db") => "Nextcloud",
        ".owncloudsync.log" => "Nextcloud",
        ".dropbox" | ".dropbox.cache" => "Dropbox",
        ".849C9593-D756-4E56-8D6E-42412F2A707B" => "OneDrive",
        ".SynologyWorkingDirectory" => "Synology Drive",
        ".tmp.drivedownload" | ".tmp.driveupload" | ".shortcut-targets-by-id" => "Google Drive",
        ".debris" => "MEGA",
        _ => return None,
    })
}

/// Folders whose children are cloud folders by location alone.
pub struct Places {
    cloud_storage: Option<PathBuf>,
    library: Option<PathBuf>,
    home: Option<PathBuf>,
}

impl Places {
    pub fn new() -> Places {
        let home = dirs::home_dir();
        Places {
            cloud_storage: home.as_ref().map(|h| h.join("Library").join("CloudStorage")),
            library: home.as_ref().map(|h| h.join("Library")),
            home,
        }
    }

    /// `dir` may contain cloud folders by location (checked once per folder a scan reads).
    pub fn special(&self, dir: &Path) -> bool {
        [&self.cloud_storage, &self.library, &self.home].iter().any(|p| p.as_deref() == Some(dir))
    }

    /// The service of `parent/name`, when its location says it's a cloud folder.
    pub fn at(&self, parent: &Path, name: &OsStr) -> Option<&'static str> {
        let n = name.to_string_lossy();
        if self.cloud_storage.as_deref() == Some(parent) {
            // macOS File Provider folders: "GoogleDrive-me@x.com", "OneDrive-Personal", "Box-Box"…
            return Some(match () {
                _ if n.starts_with("GoogleDrive") => "Google Drive",
                _ if n.starts_with("OneDrive") => "OneDrive",
                _ if n.starts_with("Dropbox") => "Dropbox",
                _ if n.starts_with("Box") => "Box",
                _ => "a cloud service",
            });
        }
        if cfg!(target_os = "macos") && self.library.as_deref() == Some(parent) && n == "Mobile Documents" {
            return Some("iCloud Drive");
        }
        if cfg!(windows) && self.home.as_deref() == Some(parent) {
            if n == "OneDrive" || n.starts_with("OneDrive - ") {
                return Some("OneDrive");
            }
            if n == "iCloudDrive" {
                return Some("iCloud Drive");
            }
        }
        None
    }

    /// The cloud folder `path` is in (or is), and its service: by location, or by a marker file
    /// in `path` or one of its parents. A few small listings; called when a folder is opened.
    pub fn root_of(&self, path: &Path) -> Option<(PathBuf, &'static str)> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        for a in path.ancestors() {
            if let (Some(parent), Some(name)) = (a.parent(), a.file_name())
                && let Some(s) = self.at(parent, name)
            {
                return Some((a.to_path_buf(), s));
            }
        }
        for a in path.ancestors() {
            if self.home.as_deref().is_some_and(|h| h.starts_with(a) && h != a) {
                break; // above the home folder: no sync client keeps its markers there
            }
            let Ok(rd) = std::fs::read_dir(a) else { continue };
            let found = rd.flatten().find_map(|e| marker(e.file_name().to_string_lossy().as_bytes()));
            if let Some(s) = found {
                return Some((a.to_path_buf(), s));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which service syncs a real folder: `NOEXPLORER_CLOUD_DIR=~/Dropbox/x cargo test cloud_dir -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cloud_dir() {
        let dir = std::env::var("NOEXPLORER_CLOUD_DIR").expect("set NOEXPLORER_CLOUD_DIR");
        let found = Places::new().root_of(Path::new(&dir));
        println!("{dir}: {found:?}");
        assert!(found.is_some());
    }

    #[test]
    fn markers_name_their_service() {
        assert_eq!(marker(b".sync_b8fb7a4a53c6.db"), Some("Nextcloud"));
        assert_eq!(marker(b"._sync_1234.db"), Some("Nextcloud"));
        assert_eq!(marker(b".dropbox"), Some("Dropbox"));
        assert_eq!(marker(b".SynologyWorkingDirectory"), Some("Synology Drive"));
        assert_eq!(marker(b".sync_b8fb7a4a53c6.db-wal"), None);
        assert_eq!(marker(b"sync_x.db"), None);
        assert_eq!(marker(b".DS_Store"), None);
    }

    #[test]
    fn locations_and_markers_find_the_cloud_root() {
        let home = std::env::temp_dir().join(format!("nx-cloud-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let home = std::fs::canonicalize(home).unwrap(); // root_of works on real paths (/var → /private/var)
        let p = Places {
            cloud_storage: Some(home.join("Library/CloudStorage")),
            library: Some(home.join("Library")),
            home: Some(home.clone()),
        };
        let gd = home.join("Library/CloudStorage/GoogleDrive-me@example.com/My Drive/x");
        std::fs::create_dir_all(&gd).unwrap();
        assert!(p.special(&home.join("Library/CloudStorage")));
        assert_eq!(p.root_of(&gd).map(|r| r.1), Some("Google Drive"));
        let nc = home.join("nc.example.com/Photos/2026");
        std::fs::create_dir_all(&nc).unwrap();
        std::fs::write(home.join("nc.example.com/.sync_abc.db"), b"").unwrap();
        let (root, s) = p.root_of(&nc).unwrap();
        assert_eq!((root.file_name().unwrap().to_str().unwrap(), s), ("nc.example.com", "Nextcloud"));
        std::fs::create_dir_all(home.join("plain/dir")).unwrap();
        assert_eq!(p.root_of(&home.join("plain/dir")), None);
        std::fs::remove_dir_all(home).unwrap();
    }
}
