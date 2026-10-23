//! Online-only cloud folders: iCloud Drive, OneDrive, Google Drive, Dropbox, Box… as the OS's
//! cloud-files layer shows them (ADR-049).
//!
//! Only *virtual* folders count: their files are placeholders that download when opened, listing a
//! folder can fetch it from the server, and access is a permission of its own (macOS File Provider,
//! Windows cloud files, Google Drive's virtual drive). A sync client that keeps real copies on this
//! disk (Nextcloud or Synology without virtual files, classic Dropbox…) is an ordinary folder.
//!
//! They're recognized by where the OS puts them, which costs nothing during a scan:
//! - macOS: `~/Library/CloudStorage/*` (OneDrive, Google Drive, Dropbox, Box, and Nextcloud with
//!   virtual files), iCloud Drive in `~/Library/Mobile Documents`, old Google Drive at
//!   `/Volumes/GoogleDrive*`;
//! - Windows: the `OneDrive*` and `iCloudDrive` folders in the home folder, and Google Drive's
//!   virtual drive (a drive whose top holds `My Drive`).
//!
//! Size scans don't enter them, rows show ☁ and the service, and the Places menu lists them in a
//! Cloud group of their own (`places`).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// A cloud place for the Places menu: where it is and what to call it.
pub struct CloudPlace {
    pub path: PathBuf,
    /// "OneDrive · Personal", "Google Drive · me@example.com", "iCloud Drive"
    pub name: String,
}

/// Where virtual cloud folders live on this computer.
#[derive(Clone)]
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

    /// Places under a stand-in home folder (tests).
    #[cfg(test)]
    pub fn rooted(home: &Path) -> Places {
        Places {
            cloud_storage: Some(home.join("Library").join("CloudStorage")),
            library: Some(home.join("Library")),
            home: Some(home.to_path_buf()),
        }
    }

    /// `dir` may contain cloud folders by location (checked once per folder a scan reads).
    pub fn special(&self, dir: &Path) -> bool {
        [&self.cloud_storage, &self.library, &self.home].iter().any(|p| p.as_deref() == Some(dir))
            || (cfg!(target_os = "macos") && dir == Path::new("/Volumes"))
    }

    /// The service of `parent/name`, when its location says it's a virtual cloud folder.
    pub fn at(&self, parent: &Path, name: &OsStr) -> Option<&'static str> {
        let n = name.to_string_lossy();
        if self.cloud_storage.as_deref() == Some(parent) {
            // File Provider domains: "OneDrive-Personal", "GoogleDrive-me@x.com", "Box-Box"…
            return Some(service_of(&n));
        }
        if cfg!(target_os = "macos") {
            if self.library.as_deref() == Some(parent) && n == "Mobile Documents" {
                return Some("iCloud Drive");
            }
            if parent == Path::new("/Volumes") && n.starts_with("GoogleDrive") {
                return Some("Google Drive"); // Drive File Stream's virtual volume
            }
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

    /// The cloud folder `path` is in (or is), and its service. Called when a folder is opened.
    pub fn root_of(&self, path: &Path) -> Option<(PathBuf, &'static str)> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        for a in path.ancestors() {
            if let (Some(parent), Some(name)) = (a.parent(), a.file_name())
                && let Some(s) = self.at(parent, name)
            {
                return Some((a.to_path_buf(), s));
            }
            if is_google_drive_letter(a) {
                return Some((a.to_path_buf(), "Google Drive"));
            }
        }
        None
    }

    /// The cloud places on this computer, for the Places menu.
    pub fn list(&self) -> Vec<CloudPlace> {
        let mut out = vec![];
        if let Some(lib) = &self.library {
            let icloud = lib.join("Mobile Documents").join("com~apple~CloudDocs");
            if cfg!(target_os = "macos") && icloud.is_dir() {
                out.push(CloudPlace { path: icloud, name: "iCloud Drive".into() });
            }
        }
        let children = |dir: &Path| -> Vec<PathBuf> {
            let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            v.sort();
            v
        };
        if let Some(cs) = &self.cloud_storage {
            for p in children(cs) {
                let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                out.push(CloudPlace { name: label(service_of(&n), &n), path: p });
            }
        }
        if cfg!(target_os = "macos") {
            for p in children(Path::new("/Volumes")) {
                if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("GoogleDrive")) {
                    out.push(CloudPlace { path: p, name: "Google Drive".into() });
                }
            }
        }
        if cfg!(windows) {
            if let Some(home) = &self.home {
                for p in children(home) {
                    let n = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    if let Some(s) = self.at(home, OsStr::new(&n)) {
                        out.push(CloudPlace { name: label(s, &n), path: p });
                    }
                }
            }
            for l in b'A'..=b'Z' {
                let root = PathBuf::from(format!("{}:\\", l as char));
                if is_google_drive_letter(&root) {
                    out.push(CloudPlace { name: format!("Google Drive ({}:)", l as char), path: root });
                }
            }
        }
        out
    }
}

/// The service named by a File Provider folder's name.
fn service_of(name: &str) -> &'static str {
    match () {
        _ if name.starts_with("GoogleDrive") => "Google Drive",
        _ if name.starts_with("OneDrive") => "OneDrive",
        _ if name.starts_with("Dropbox") => "Dropbox",
        _ if name.starts_with("Box") => "Box",
        _ if name.starts_with("Nextcloud") => "Nextcloud",
        _ => "Cloud",
    }
}

/// "OneDrive-Personal" → "OneDrive · Personal"; "GoogleDrive-me@x.com" → "Google Drive · me@x.com".
fn label(service: &str, folder: &str) -> String {
    let rest = folder.split_once('-').map(|(_, r)| r).or_else(|| folder.strip_prefix("OneDrive - ")).unwrap_or("");
    let rest = rest.trim();
    if rest.is_empty() || rest == service || (service == "Box" && rest == "Box") {
        service.to_string()
    } else {
        format!("{service} · {rest}")
    }
}

/// Google Drive's virtual drive on Windows: a drive whose top holds "My Drive".
fn is_google_drive_letter(p: &Path) -> bool {
    cfg!(windows) && p.parent().is_none() && p.join("My Drive").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which service a real folder belongs to: `NOEXPLORER_CLOUD_DIR=~/Library/CloudStorage/x cargo test cloud_dir -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cloud_dir() {
        let dir = std::env::var("NOEXPLORER_CLOUD_DIR").expect("set NOEXPLORER_CLOUD_DIR");
        println!("{dir}: {:?}", Places::new().root_of(Path::new(&dir)));
        for p in Places::new().list() {
            println!("place: {} = {}", p.name, p.path.display());
        }
    }

    #[test]
    fn labels_read_like_finder() {
        assert_eq!(label("OneDrive", "OneDrive-Personal"), "OneDrive · Personal");
        assert_eq!(label("OneDrive", "OneDrive - Contoso"), "OneDrive · Contoso");
        assert_eq!(label("Google Drive", "GoogleDrive-me@example.com"), "Google Drive · me@example.com");
        assert_eq!(label("Box", "Box-Box"), "Box");
        assert_eq!(label("Dropbox", "Dropbox"), "Dropbox");
        assert_eq!(service_of("Nextcloud-dev@cloud.example.com"), "Nextcloud");
    }

    #[test]
    fn virtual_cloud_folders_are_found_by_location_only() {
        let home = std::env::temp_dir().join(format!("nx-cloud-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let home = std::fs::canonicalize(home).unwrap(); // root_of works on real paths (/var → /private/var)
        let p = Places {
            cloud_storage: Some(home.join("Library/CloudStorage")),
            library: Some(home.join("Library")),
            home: Some(home.clone()),
        };
        let od = home.join("Library/CloudStorage/OneDrive-Personal/Documents/x");
        std::fs::create_dir_all(&od).unwrap();
        assert!(p.special(&home.join("Library/CloudStorage")));
        assert_eq!(p.root_of(&od).map(|r| r.1), Some("OneDrive"));
        assert!(p.list().iter().any(|c| c.name == "OneDrive · Personal"));
        // A sync client that keeps real files (Nextcloud without virtual files) is a plain folder.
        let nc = home.join("nc.example.com/Photos");
        std::fs::create_dir_all(&nc).unwrap();
        std::fs::write(home.join("nc.example.com/.sync_abc.db"), b"").unwrap();
        assert_eq!(p.root_of(&nc), None);
        std::fs::remove_dir_all(home).unwrap();
    }
}
