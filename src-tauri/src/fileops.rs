//! Rename, move and duplicate (experimental `file_ops`), on this computer and on remotes.
//!
//! Local operations are plain renames on the same disk and never overwrite: a name that's taken is
//! an error, and duplicates get "name copy.ext". Moving to another disk would mean deleting the
//! original, which the app never does (ADR-006), so it's refused. Remote operations run on the
//! server (`Remote::transfer_within`) as a background job of kind "move" or "dup".

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::jobs::Job;
use crate::state::AppState;
use crate::util::{file_name, job_label};

/// Result of a local operation: what was done (for undo) and what failed.
#[derive(Serialize, Default)]
pub struct OpResult {
    /// `[from, to]` absolute paths.
    pub done: Vec<[String; 2]>,
    pub errors: Vec<String>,
}

fn check_name(name: &str) -> Result<&str, String> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\']) {
        return Err("That name can't be used".into());
    }
    Ok(n)
}

/// Same-disk rename that refuses to replace anything.
fn rename_no_clobber(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!("“{}” already exists", to.file_name().unwrap_or_default().to_string_lossy()));
    }
    if from.is_dir() && to.starts_with(from) {
        return Err("A folder can't go inside itself".into());
    }
    std::fs::rename(from, to).map_err(|e| match e.kind() {
        std::io::ErrorKind::CrossesDevices => "Moving to another disk isn't supported yet: copy it instead".into(),
        _ => e.to_string(),
    })
}

#[tauri::command]
pub fn rename_local(path: String, name: String) -> Result<String, String> {
    let from = PathBuf::from(&path);
    let to = from.with_file_name(check_name(&name)?);
    rename_no_clobber(&from, &to)?;
    Ok(to.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn move_local(paths: Vec<String>, dest: String) -> OpResult {
    let mut r = OpResult::default();
    for p in paths {
        let from = PathBuf::from(&p);
        let to = Path::new(&dest).join(from.file_name().unwrap_or_default());
        match rename_no_clobber(&from, &to) {
            Ok(()) => r.done.push([p, to.to_string_lossy().into_owned()]),
            Err(e) => r.errors.push(format!("{}: {e}", file_name(&p.replace('\\', "/")))),
        }
    }
    r
}

/// "name copy.ext", then "name copy 2.ext", … — the first free one next to `p`.
fn copy_name(p: &Path) -> PathBuf {
    let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = if p.is_dir() { None } else { p.extension().map(|e| e.to_string_lossy().into_owned()) };
    (1..)
        .map(|i| {
            let base = if i == 1 { format!("{stem} copy") } else { format!("{stem} copy {i}") };
            p.with_file_name(match &ext {
                Some(e) => format!("{base}.{e}"),
                None => base,
            })
        })
        .find(|c| !c.exists())
        .unwrap()
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            // Symlinks are skipped, like uploads do: never follow them into other places.
            if e.file_type()?.is_symlink() {
                continue;
            }
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

#[tauri::command]
pub async fn duplicate_local(paths: Vec<String>) -> OpResult {
    tauri::async_runtime::spawn_blocking(move || {
        let mut r = OpResult::default();
        for p in paths {
            let from = PathBuf::from(&p);
            let to = copy_name(&from);
            match copy_tree(&from, &to) {
                Ok(()) => r.done.push([p, to.to_string_lossy().into_owned()]),
                Err(e) => r.errors.push(format!("{}: {e}", file_name(&p.replace('\\', "/")))),
            }
        }
        r
    })
    .await
    .unwrap_or_default()
}

/// Remote rename / move (`copy` false) or duplicate (`copy` true): `pairs` are `[from, to]` keys in
/// one container (folders end with `/`). Runs as a background job; the UI refreshes on its end.
#[tauri::command]
pub async fn remote_move(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    pairs: Vec<[String; 2]>,
    copy: bool,
) -> Result<String, String> {
    let r = state.remote(account.as_deref())?;
    let first = pairs.first().ok_or("Nothing to do")?;
    // The folder the targets land in, so the views showing it refresh.
    let to = first[1].trim_end_matches('/');
    let prefix = to.rfind('/').map(|i| to[..=i].to_string()).unwrap_or_default();
    let label = job_label(pairs.iter().map(|p| file_name(&p[0])).collect());
    let job = Job::new(&app, if copy { "dup" } else { "move" }, label, r.account(), &bucket, &prefix);
    let id = job.id.clone();
    let j = job.clone();
    job.spawn(app, async move {
        for [from, to] in pairs {
            if j.cancelled() {
                break;
            }
            if let Err(e) = r.transfer_within(&bucket, &from, &to, copy, Some(&j)).await {
                j.fail(&file_name(&from), e);
            }
        }
    });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_rename_move_duplicate_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("nx-fileops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        let p = |n: &str| dir.join(n).to_string_lossy().into_owned();

        assert!(rename_local(p("a.txt"), "b.txt".into()).is_err(), "never replaces");
        assert!(rename_local(p("a.txt"), "x/y".into()).is_err());
        assert_eq!(rename_local(p("a.txt"), "c.txt".into()).unwrap(), p("c.txt"));

        let d = tauri::async_runtime::block_on(duplicate_local(vec![p("c.txt"), p("c.txt")]));
        assert_eq!(d.done[0][1], p("c copy.txt"));
        assert_eq!(d.done[1][1], p("c copy 2.txt"));

        std::fs::write(dir.join("sub/b.txt"), "taken").unwrap();
        let m = move_local(vec![p("b.txt"), p("c.txt")], p("sub"));
        assert_eq!(m.done.len(), 1);
        assert_eq!(m.errors.len(), 1, "b.txt exists in sub");
        assert_eq!(std::fs::read_to_string(dir.join("sub/b.txt")).unwrap(), "taken");
        assert!(move_local(vec![p("sub")], p("sub")).errors.len() == 1, "not into itself");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
