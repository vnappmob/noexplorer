//! Folder sizes for the local pane: a live index on top of the scanner in `scan.rs` (ADR-022).
//!
//! - `folder_sizes(path, dirs, auto)` is called whenever the local pane shows a folder. It
//!   answers from the index right away and, if needed, scans `path` in the background. Totals
//!   stream to the UI as `"dir-sizes"` events (batched every ~100 ms) as each subfolder finishes.
//! - Scanned roots stay indexed and are watched (FSEvents / ReadDirectoryChangesW / inotify), so
//!   going back to a folder is instant and sizes follow changes: only the changed folder is
//!   re-read and the difference is applied to its ancestors. When the shown folder's own entries
//!   change, the event says `relist` and the pane lists it again (live folder view).
//! - Automatic scans are polite: low-priority threads, never another volume, and on macOS they
//!   skip folders that would trigger a privacy prompt (Desktop, Documents, Downloads, ~/Library…)
//!   unless the user has already opened them in NoExplorer. The Σ button scans everything.
//! - The last totals of folders the pane has shown are kept across launches (`Seen`, in the OS
//!   cache folder). Opening a folder the index doesn't cover yet (after a restart, or once its root
//!   was evicted) shows those at once, marked `stale`, while the scan refreshes them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::scan::{self, Options, Sink, Totals};

/// Indexed roots kept (and watched) at once; the least recently used one is dropped.
const MAX_ROOTS: usize = 6;
/// Upper bound on indexed folders across roots (≈ 100 bytes each).
const MAX_DIRS: usize = 1_500_000;

/// Folders whose last totals are remembered across launches (≈ 100 bytes each on disk).
const MAX_SEEN: usize = 20_000;

#[derive(Serialize, Clone)]
pub struct DirSize {
    path: String,
    #[serde(flatten)]
    totals: Totals,
    /// From a previous scan (remembered), not the live index: a refresh may change it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stale: bool,
}

/// Payload of the `"dir-sizes"` event and the `folder_sizes` result. Mirrors `DirSizes` in `src/types.ts`.
#[derive(Serialize, Clone)]
pub struct DirSizes {
    /// The folder these sizes belong to (the local pane's folder when sent).
    folder: String,
    /// Totals of subfolders of `folder`.
    sizes: Vec<DirSize>,
    /// Total of `folder` itself, once known.
    total: Option<Totals>,
    /// `total` is remembered from a previous scan (see `DirSize::stale`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    total_stale: bool,
    /// A scan that will report more is running.
    scanning: bool,
    /// Entries of `folder` itself changed on disk (added, removed, renamed): list it again.
    relist: bool,
}

struct Root {
    path: PathBuf,
    /// Automatic scans skip protected folders; manual (Σ) ones don't.
    auto: bool,
    _watcher: Option<notify::RecommendedWatcher>,
}

struct Running {
    root: PathBuf,
    auto: bool,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Inner {
    /// Totals of every indexed folder.
    map: HashMap<PathBuf, Totals>,
    /// Indexed + watched roots, least recently used first.
    roots: Vec<Root>,
    running: Option<Running>,
    /// Folder shown in the local pane: only its subfolders are reported.
    focus: Option<PathBuf>,
    /// Folders finished since the last event.
    dirty: Vec<PathBuf>,
    /// The focus folder's own entries changed since the last event.
    relist: bool,
}

#[derive(Default)]
pub struct SizeIndex {
    inner: Mutex<Inner>,
    wake: Condvar,
    /// macOS privacy-protected folders the user has opened in the app (so access is granted).
    /// Kept in `<app config dir>/granted.json`.
    granted: Mutex<HashSet<PathBuf>>,
    granted_file: Option<PathBuf>,
    seen: Mutex<Seen>,
}

/// Last known totals of folders the local pane has shown, kept in `<OS cache dir>/folder-sizes.json`.
#[derive(Default)]
struct Seen {
    /// Totals and when they were last seen fresh (Unix seconds).
    map: HashMap<PathBuf, (Totals, u64)>,
    file: Option<PathBuf>,
    changed: bool,
    saved: Option<Instant>,
}

impl Seen {
    fn load(file: Option<PathBuf>) -> Seen {
        let entries: Vec<(PathBuf, Totals, u64)> = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Seen { map: entries.into_iter().map(|(p, t, at)| (p, (t, at))).collect(), file, ..Default::default() }
    }

    fn remember(&mut self, items: impl IntoIterator<Item = (PathBuf, Totals)>) {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        for (p, t) in items {
            self.map.insert(p, (t, now));
            self.changed = true;
        }
    }

    /// Writes the file (oldest folders dropped beyond [`MAX_SEEN`]), at most every 10 s unless `now`.
    fn save(&mut self, now: bool) {
        if !self.changed || (!now && self.saved.is_some_and(|t| t.elapsed() < Duration::from_secs(10))) {
            return;
        }
        if self.map.len() > MAX_SEEN {
            let mut by_age: Vec<(PathBuf, u64)> = self.map.iter().map(|(p, (_, at))| (p.clone(), *at)).collect();
            by_age.sort_by_key(|(_, at)| *at);
            for (p, _) in by_age.into_iter().take(self.map.len() - MAX_SEEN * 9 / 10) {
                self.map.remove(&p);
            }
        }
        self.changed = false;
        self.saved = Some(Instant::now());
        let Some(f) = &self.file else { return };
        let entries: Vec<(&PathBuf, &Totals, &u64)> = self.map.iter().map(|(p, (t, at))| (p, t, at)).collect();
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = f.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&entries).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, f);
        }
    }
}

impl Sink for SizeIndex {
    fn done(&self, path: &Path, t: Totals) {
        let mut g = self.inner.lock().unwrap();
        g.map.insert(path.to_path_buf(), t);
        if g.focus.as_deref().is_some_and(|f| path == f || path.parent() == Some(f)) {
            g.dirty.push(path.to_path_buf());
            self.wake.notify_one();
        }
    }
}

// ---------- policy ----------

/// Folders an automatic scan doesn't enter, unless the scan starts inside them or the user has
/// opened them (macOS shows a privacy prompt, or the folder is large and rarely interesting).
fn protected() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return vec![];
    }
    let Some(home) = dirs::home_dir() else { return vec![] };
    ["Desktop", "Documents", "Downloads", "Library", "Pictures/Photos Library.photoslibrary"]
        .iter()
        .map(|p| home.join(p))
        .chain(
            ["Containers", "Group Containers", "Mobile Documents", "CloudStorage", "Mail", "Messages", "Safari"]
                .iter()
                .map(|p| home.join("Library").join(p)),
        )
        .collect()
}

/// An automatic scan may run here: same volume as the home folder (no external or network disks).
fn auto_allowed(path: &Path) -> bool {
    dirs::home_dir().is_some_and(|home| same_volume(path, &home))
}

#[cfg(windows)]
fn same_volume(a: &Path, b: &Path) -> bool {
    let drive = |p: &Path| p.components().next().map(|c| c.as_os_str().to_ascii_lowercase());
    drive(a) == drive(b)
}

#[cfg(not(windows))]
fn same_volume(a: &Path, b: &Path) -> bool {
    scan::dev_of(a).is_some() && scan::dev_of(a) == scan::dev_of(b)
}

/// Drops the macOS privacy-prompt risk: lower thread priority for scans.
fn background_priority() {
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(qos: u32, relative: i32) -> i32;
        }
        const QOS_CLASS_UTILITY: u32 = 0x11;
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
    }
}

fn threads() -> usize {
    // APFS/ext4 stop scaling around 6 parallel readers (measured, ADR-022).
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 6)
}

impl SizeIndex {
    /// Remembers that the user opened `path` (called by `list_local`): if it's in a protected
    /// folder, automatic scans may enter that folder from now on.
    pub fn opened(&self, path: &Path) {
        let mut granted = self.granted.lock().unwrap();
        let mut changed = false;
        for p in protected() {
            if path.starts_with(&p) {
                changed |= granted.insert(p);
            }
        }
        if changed && let Some(f) = &self.granted_file {
            let _ = std::fs::write(f, serde_json::to_vec(&*granted).unwrap_or_default());
        }
    }

    pub(crate) fn enter_policy(&self, root: &Path, auto: bool) -> impl Fn(&Path) -> bool + Sync + use<> {
        let skip: Vec<PathBuf> = if auto {
            let granted = self.granted.lock().unwrap();
            protected().into_iter().filter(|p| !root.starts_with(p) && !granted.contains(p)).collect()
        } else {
            vec![]
        };
        move |p: &Path| !skip.iter().any(|s| s == p)
    }

    /// What's known about `folder` and its `dirs` right now: the index first, then the totals
    /// remembered from earlier scans (marked stale).
    fn snapshot(&self, folder: &Path, dirs: &[PathBuf]) -> DirSizes {
        let g = self.inner.lock().unwrap();
        let mut seen = self.seen.lock().unwrap();
        seen.remember(dirs.iter().chain([&folder.to_path_buf()]).filter_map(|d| g.map.get(d).map(|t| (d.clone(), *t))));
        let get = |d: &Path| match g.map.get(d) {
            Some(t) => Some((*t, false)),
            None => seen.map.get(d).map(|(t, _)| (*t, true)),
        };
        let sizes = dirs
            .iter()
            .filter_map(|d| get(d).map(|(t, stale)| DirSize { path: d.to_string_lossy().into(), totals: t, stale }))
            .collect();
        let total = get(folder);
        DirSizes {
            folder: folder.to_string_lossy().into(),
            sizes,
            total: total.map(|t| t.0),
            total_stale: total.is_some_and(|t| t.1),
            scanning: g.running.as_ref().is_some_and(|r| folder.starts_with(&r.root)),
            relist: false,
        }
    }

    fn mark_used(g: &mut Inner, path: &Path) {
        if let Some(i) = g.roots.iter().position(|r| path.starts_with(&r.path)) {
            let r = g.roots.remove(i);
            g.roots.push(r);
        }
    }

    /// Covered by a finished, watched root.
    fn indexed(g: &Inner, path: &Path) -> bool {
        g.map.contains_key(path) && g.roots.iter().any(|r| path.starts_with(&r.path))
    }
}

/// Starts a background scan of `root` (cancelling one that doesn't cover it).
fn start_scan(app: &AppHandle, root: PathBuf, auto: bool) {
    let idx = app.state::<SizeIndex>();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut g = idx.inner.lock().unwrap();
        if let Some(r) = &g.running {
            // Already part of a bigger scan (a polite one doesn't cover a Σ request).
            if root.starts_with(&r.root) && (auto || !r.auto) {
                return;
            }
            r.cancel.store(true, Relaxed);
        }
        g.running = Some(Running { root: root.clone(), auto, cancel: cancel.clone() });
    }
    let app = app.clone();
    std::thread::spawn(move || {
        background_priority();
        let idx = app.state::<SizeIndex>();
        let enter = idx.enter_policy(&root, auto);
        // Watched roots below this one are already exact: reuse them instead of descending.
        let known_roots: HashMap<PathBuf, Totals> = {
            let g = idx.inner.lock().unwrap();
            g.roots
                .iter()
                .filter(|r| r.path != root && r.path.starts_with(&root))
                .filter_map(|r| g.map.get(&r.path).map(|t| (r.path.clone(), *t)))
                .collect()
        };
        let known = |p: &Path| known_roots.get(p).copied();
        let opts = Options { enter: &enter, known: &known, cancel: &cancel, threads: threads() };
        let result = scan::scan(&root, &opts, &*idx);
        let mut g = idx.inner.lock().unwrap();
        if g.running.as_ref().is_some_and(|r| Arc::ptr_eq(&r.cancel, &cancel)) {
            g.running = None;
        }
        if result.is_some() {
            // Roots inside this one are now covered by its watcher.
            g.roots.retain(|r| !r.path.starts_with(&root));
            let watcher = watch(&app, &root);
            g.roots.push(Root { path: root.clone(), auto, _watcher: watcher });
            evict(&mut g);
        }
        if g.focus.as_deref().is_some_and(|f| f.starts_with(&root)) {
            let f = g.focus.clone().unwrap();
            g.dirty.push(f); // final event: scanning = false
            idx.wake.notify_one();
        }
    });
}

/// Drops the least recently used roots (and their folders) beyond the limits.
fn evict(g: &mut Inner) {
    while g.roots.len() > MAX_ROOTS || (g.map.len() > MAX_DIRS && g.roots.len() > 1) {
        let old = g.roots.remove(0);
        let still = |p: &Path| g.roots.iter().any(|r| p.starts_with(&r.path));
        let gone: Vec<PathBuf> = g.map.keys().filter(|p| p.starts_with(&old.path) && !still(p)).cloned().collect();
        for p in gone {
            g.map.remove(&p);
        }
    }
}

// ---------- live updates ----------

fn watch(app: &AppHandle, root: &Path) -> Option<notify::RecommendedWatcher> {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut w = notify::recommended_watcher(tx).ok()?;
    // FSEvents and ReadDirectoryChangesW watch a tree natively. inotify needs one watch per folder
    // (a low system limit), so Linux only follows the root's own entries.
    let mode = if cfg!(target_os = "linux") { RecursiveMode::NonRecursive } else { RecursiveMode::Recursive };
    w.watch(root, mode).ok()?;
    let app = app.clone();
    std::thread::spawn(move || {
        background_priority();
        // Coalesce bursts (a download writes many times) into one update per folder.
        while let Ok(first) = rx.recv() {
            let mut changed: HashSet<PathBuf> = HashSet::new();
            let mut add = |ev: notify::Result<notify::Event>| {
                if let Ok(ev) = ev {
                    for p in ev.paths {
                        let rescan = ev.attrs.flag() == Some(notify::event::Flag::Rescan);
                        if let Some(d) = if rescan { Some(p.clone()) } else { p.parent().map(Path::to_path_buf) } {
                            changed.insert(d);
                        }
                    }
                }
            };
            add(first);
            std::thread::sleep(Duration::from_millis(400));
            while let Ok(ev) = rx.try_recv() {
                add(ev);
            }
            refresh_dirs(&app, changed);
        }
    });
    Some(w)
}

/// Re-reads changed folders (deepest first) and applies the difference up the tree.
fn refresh_dirs(app: &AppHandle, changed: HashSet<PathBuf>) {
    let idx = app.state::<SizeIndex>();
    let mut dirs: Vec<PathBuf> = {
        let g = idx.inner.lock().unwrap();
        changed.into_iter().filter(|d| g.map.contains_key(d)).collect()
    };
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let auto = {
            let g = idx.inner.lock().unwrap();
            g.roots.iter().find(|r| d.starts_with(&r.path)).map(|r| r.auto)
        };
        let Some(auto) = auto else { continue };
        let root_hint = d.clone();
        let enter = idx.enter_policy(&root_hint, auto);
        let Some(new) = reread(&idx, &d, &enter) else {
            // The folder is gone: its parent's update removes it.
            continue;
        };
        let mut g = idx.inner.lock().unwrap();
        let old = g.map.insert(d.clone(), new).unwrap_or_default();
        // Apply the difference to every indexed ancestor up to the root.
        let roots: Vec<PathBuf> = g.roots.iter().map(|r| r.path.clone()).collect();
        let mut a = d.parent();
        while let Some(p) = a {
            if !roots.iter().any(|r| p.starts_with(r)) {
                break;
            }
            if let Some(t) = g.map.get_mut(p) {
                *t = apply(*t, old, new);
            }
            a = p.parent();
        }
        let focus = g.focus.clone();
        if let Some(f) = focus
            && (d.starts_with(&f) || f.starts_with(&d))
        {
            // Report the focus folder's child that contains `d` (or `d` itself) and the total.
            let child = d.ancestors().find(|x| x.parent() == Some(&f)).map(Path::to_path_buf);
            g.relist |= d == f;
            g.dirty.extend(child.into_iter().chain([f]));
            idx.wake.notify_one();
        }
    }
}

/// New totals of `d`: its own files re-read, subfolders from the index (new ones are scanned).
fn reread(idx: &SizeIndex, d: &Path, enter: &(dyn Fn(&Path) -> bool + Sync)) -> Option<Totals> {
    let cancel = AtomicBool::new(false);
    let mut buf = vec![0u8; 64 * 1024];
    let mut own = Totals::default();
    let mut subdirs = vec![];
    let dev = scan::dev_of(d)?;
    scan::read_dir(d, &mut buf, &mut |e| {
        if e.dir && (e.dev == 0 || e.dev == dev) {
            subdirs.push(d.join(bytes_name(e.name)));
        } else if e.file {
            own.bytes += e.size;
            own.files += 1;
        }
    })
    .ok()?;
    let present: HashSet<&PathBuf> = subdirs.iter().collect();
    {
        // Forget subfolders that disappeared.
        let mut g = idx.inner.lock().unwrap();
        let gone: Vec<PathBuf> =
            g.map.keys().filter(|p| p.parent() == Some(d) && !present.contains(p)).cloned().collect();
        for p in gone {
            let under: Vec<PathBuf> = g.map.keys().filter(|k| k.starts_with(&p)).cloned().collect();
            for k in under {
                g.map.remove(&k);
            }
        }
    }
    for s in &subdirs {
        own.dirs += 1;
        if !enter(s) {
            own.skipped += 1;
            continue;
        }
        let known = idx.inner.lock().unwrap().map.get(s).copied();
        let t = match known {
            Some(t) => t,
            None => {
                let o = Options { enter, known: &|_| None, cancel: &cancel, threads: 2 };
                scan::scan(s, &o, idx).unwrap_or(Totals { skipped: 1, ..Default::default() })
            }
        };
        own.bytes += t.bytes;
        own.files += t.files;
        own.dirs += t.dirs;
        own.skipped += t.skipped;
    }
    Some(own)
}

fn apply(t: Totals, old: Totals, new: Totals) -> Totals {
    let f = |v: u64, o: u64, n: u64| (v as i128 - o as i128 + n as i128).max(0) as u64;
    Totals {
        bytes: f(t.bytes, old.bytes, new.bytes),
        files: f(t.files, old.files, new.files),
        dirs: f(t.dirs, old.dirs, new.dirs),
        skipped: f(t.skipped as u64, old.skipped as u64, new.skipped as u64) as u32,
    }
}

#[cfg(unix)]
fn bytes_name(b: &[u8]) -> &std::ffi::OsStr {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(b)
}

#[cfg(not(unix))]
fn bytes_name(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

// ---------- events ----------

/// Sends batched `"dir-sizes"` events for the focus folder (one thread for the app's lifetime,
/// asleep until something finishes).
pub fn setup(app: &AppHandle) {
    let granted_file = app.path().app_config_dir().ok().map(|d| d.join("granted.json"));
    let granted: HashSet<PathBuf> = granted_file
        .as_ref()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let seen = Seen::load(app.path().app_cache_dir().ok().map(|d| d.join("folder-sizes.json")));
    app.manage(SizeIndex { granted: Mutex::new(granted), granted_file, seen: Mutex::new(seen), ..Default::default() });
    let app = app.clone();
    std::thread::spawn(move || {
        let idx = app.state::<SizeIndex>();
        loop {
            {
                let mut g = idx.inner.lock().unwrap();
                while g.dirty.is_empty() {
                    g = idx.wake.wait(g).unwrap();
                }
            }
            std::thread::sleep(Duration::from_millis(100)); // batch what finishes meanwhile
            let payload = {
                let mut g = idx.inner.lock().unwrap();
                let dirty = std::mem::take(&mut g.dirty);
                let relist = std::mem::take(&mut g.relist);
                let Some(f) = g.focus.clone() else { continue };
                let mut seen = HashSet::new();
                let sizes = dirty
                    .iter()
                    .filter(|p| p.parent() == Some(&f) && seen.insert(*p))
                    .filter_map(|p| {
                        g.map.get(p).map(|t| DirSize { path: p.to_string_lossy().into(), totals: *t, stale: false })
                    })
                    .collect();
                DirSizes {
                    folder: f.to_string_lossy().into(),
                    sizes,
                    total: g.map.get(&f).copied(),
                    total_stale: false,
                    scanning: g.running.as_ref().is_some_and(|r| f.starts_with(&r.root)),
                    relist,
                }
            };
            {
                // Fresh totals of what the pane shows are remembered for the next launch.
                let mut seen = idx.seen.lock().unwrap();
                let fresh = payload.sizes.iter().map(|d| (PathBuf::from(&d.path), d.totals));
                seen.remember(fresh.chain(payload.total.map(|t| (PathBuf::from(&payload.folder), t))));
                seen.save(!payload.scanning);
            }
            let _ = app.emit("dir-sizes", payload);
        }
    });
}

// ---------- commands ----------

/// Sizes of `dirs` (subfolders of `path`, as listed) and of `path`, from the index. Starts a
/// background scan when they aren't known; results follow as `"dir-sizes"` events.
/// `auto` = the automatic scan on opening a folder (polite, see the module docs); false = Σ.
#[tauri::command]
pub fn folder_sizes(
    app: AppHandle,
    idx: State<'_, SizeIndex>,
    path: String,
    dirs: Vec<String>,
    auto: bool,
) -> DirSizes {
    let folder = PathBuf::from(&path);
    let dirs: Vec<PathBuf> = dirs.into_iter().map(PathBuf::from).collect();
    let need = {
        let mut g = idx.inner.lock().unwrap();
        g.focus = Some(folder.clone());
        g.dirty.clear();
        g.relist = false;
        SizeIndex::mark_used(&mut g, &folder);
        let partial = g.map.get(&folder).is_some_and(|t| t.skipped > 0);
        !SizeIndex::indexed(&g, &folder) || (!auto && partial)
    };
    if need && (!auto || auto_allowed(&folder)) {
        if !auto {
            // Σ: forget the polite totals of this subtree so it's rescanned in full.
            let mut g = idx.inner.lock().unwrap();
            g.roots.retain(|r| !r.path.starts_with(&folder));
        }
        start_scan(&app, folder.clone(), auto);
    }
    idx.snapshot(&folder, &dirs)
}

/// Drops the focus (the local pane left this folder, or auto sizes were turned off).
#[tauri::command]
pub fn stop_sizes(idx: State<'_, SizeIndex>) {
    let mut g = idx.inner.lock().unwrap();
    g.focus = None;
    if let Some(r) = g.running.take() {
        r.cancel.store(true, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_sizes_survive_a_restart_and_stay_bounded() {
        let dir = std::env::temp_dir().join(format!("nx-seen-{}", uuid::Uuid::new_v4()));
        let file = dir.join("folder-sizes.json");
        let t = |b| Totals { bytes: b, files: 1, dirs: 0, skipped: 0 };
        let mut s = Seen::load(Some(file.clone()));
        s.remember([(PathBuf::from("/a"), t(10)), (PathBuf::from("/a/b"), t(4))]);
        s.save(true);
        let again = Seen::load(Some(file.clone()));
        assert_eq!(again.map.get(Path::new("/a/b")).map(|e| e.0), Some(t(4)));
        let mut big = Seen::load(Some(file.clone()));
        big.remember((0..MAX_SEEN + 5).map(|i| (PathBuf::from(format!("/x/{i}")), t(i as u64))));
        big.save(true);
        assert!(Seen::load(Some(file)).map.len() <= MAX_SEEN, "pruned to the limit");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn apply_moves_totals_by_the_difference() {
        let t = |b, f| Totals { bytes: b, files: f, dirs: 0, skipped: 0 };
        assert_eq!(apply(t(100, 10), t(30, 3), t(50, 4)), t(120, 11));
        assert_eq!(apply(t(10, 1), t(30, 3), t(0, 0)), t(0, 0), "never below zero");
    }

    #[test]
    fn evict_drops_least_recently_used_roots_and_their_folders() {
        let mut g = Inner::default();
        for i in 0..MAX_ROOTS + 1 {
            let p = PathBuf::from(format!("/r{i}"));
            g.map.insert(p.join("sub"), Totals::default());
            g.map.insert(p.clone(), Totals::default());
            g.roots.push(Root { path: p, auto: true, _watcher: None });
        }
        evict(&mut g);
        assert_eq!(g.roots.len(), MAX_ROOTS);
        assert!(!g.map.contains_key(Path::new("/r0")) && !g.map.contains_key(Path::new("/r0/sub")));
        assert!(g.map.contains_key(Path::new("/r1/sub")));
    }
}
