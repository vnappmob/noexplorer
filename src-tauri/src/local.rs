//! Local filesystem commands for the left pane. Intentionally read-mostly: the app never
//! deletes or overwrites local files (see ADR-006).

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncReadExt;

use crate::remote::Entry;

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
pub async fn list_local(path: Option<String>, hidden: bool) -> Result<LocalListing, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
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

        let l = list_local(path.clone(), false).await.unwrap();
        let mut names: Vec<_> = l.entries.iter().map(|e| (e.name.as_str(), e.dir, e.size)).collect();
        names.sort();
        assert_eq!(names, [("a.txt", false, 3), ("sub", true, 0)]);
        assert_eq!(l.crumbs.last().unwrap().path, l.path);

        let l = list_local(path, true).await.unwrap();
        assert_eq!(l.entries.len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn mkdir_local_rejects_bad_names() {
        for bad in ["", "..", "a/b", "a\\b"] {
            assert!(mkdir_local("/tmp".into(), bad.into()).await.is_err(), "{bad:?}");
        }
    }
}
