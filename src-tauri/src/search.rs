//! Search: every file and folder below a folder whose name matches, on this computer or a remote.
//!
//! Queries: words that must all be in the name (any case), `*` / `?` wildcards (`*.pdf`,
//! `IMG_????.jpg`), and `ext:pdf`. Results stream to the UI as `"search"` events every 100 ms, so
//! the first ones show while the walk goes on; [`MAX_HITS`] stops it.
//!
//! This computer: our own parallel walker on `scan::read_dir` (one `getattrlistbulk` call per batch
//! of entries on macOS, like the size scanner), several folders at once, staying on one disk and
//! skipping the same macOS privacy-protected folders as automatic size scans until they've been
//! opened (`sizes.rs`), so a search never triggers a permission prompt. Hidden files only when the
//! pane shows them. Remotes: the provider's full listing below the folder (`Remote::list_files`),
//! with folders taken from the paths.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::provider::Entry;
use crate::scan;
use crate::state::AppState;

/// A search stops after this many results (the list stays usable; refine the query for more).
pub const MAX_HITS: usize = 10_000;
const FLUSH: Duration = Duration::from_millis(100);

static RUNNING: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = LazyLock::new(Default::default);

/// One batch of results (`"search"` event). The last one has `done`.
#[derive(Serialize, Clone, Debug, Default)]
pub struct SearchEvent {
    pub id: String,
    pub hits: Vec<Entry>,
    pub done: bool,
    /// Folders read and files seen so far.
    pub dirs: u64,
    pub files: u64,
    /// Folders not entered: unreadable, other disks, or private folders not opened yet.
    pub skipped: u64,
    /// Stopped at `MAX_HITS`.
    pub capped: bool,
    pub error: Option<String>,
}

// ---------- the query ----------

enum Term {
    Sub(String),
    Glob(Vec<char>),
    Ext(String),
}

pub struct Matcher {
    terms: Vec<Term>,
}

impl Matcher {
    /// None for an empty query.
    pub fn new(q: &str) -> Option<Matcher> {
        let terms: Vec<Term> = q
            .split_whitespace()
            .map(|t| {
                let t = t.to_lowercase();
                if let Some(e) = t.strip_prefix("ext:") {
                    Term::Ext(e.trim_start_matches('.').to_string())
                } else if t.contains(['*', '?']) {
                    Term::Glob(t.chars().collect())
                } else {
                    Term::Sub(t)
                }
            })
            .collect();
        (!terms.is_empty()).then_some(Matcher { terms })
    }

    pub fn matches(&self, name: &str) -> bool {
        let n = name.to_lowercase();
        self.terms.iter().all(|t| match t {
            Term::Sub(s) => n.contains(s.as_str()),
            Term::Ext(e) => n.rsplit_once('.').is_some_and(|(_, x)| x == e),
            Term::Glob(p) => glob(p, &n.chars().collect::<Vec<_>>()),
        })
    }
}

/// `*` any run, `?` one character; the whole name must match.
fn glob(p: &[char], s: &[char]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, si));
            pi += 1;
        } else if let Some((sp, ss)) = star {
            pi = sp + 1;
            si = ss + 1;
            star = Some((sp, ss + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

// ---------- this computer ----------

struct Walk<'a> {
    m: &'a Matcher,
    hidden: bool,
    enter: &'a (dyn Fn(&Path) -> bool + Sync),
    cancel: &'a AtomicBool,
    root_dev: u64,
    queue: Mutex<(VecDeque<PathBuf>, usize)>,
    wake: Condvar,
    out: Mutex<Vec<Entry>>,
    hits: AtomicUsize,
    dirs: AtomicU64,
    files: AtomicU64,
    skipped: AtomicU64,
}

fn millis(p: &Path) -> Option<i64> {
    let t = std::fs::symlink_metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

impl Walk<'_> {
    fn hit(&self, path: &Path, name: String, dir: bool, size: u64) {
        if self.hits.fetch_add(1, AcqRel) >= MAX_HITS {
            self.cancel.store(true, Release);
            return;
        }
        let key = path.to_string_lossy().into_owned();
        self.out.lock().unwrap().push(Entry { key, name, size, modified: millis(path), dir, hash: None });
    }

    fn worker(&self) {
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let dir = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if self.cancel.load(Acquire) {
                        self.wake.notify_all();
                        return;
                    }
                    if let Some(d) = q.0.pop_front() {
                        q.1 += 1;
                        break d;
                    }
                    if q.1 == 0 {
                        self.wake.notify_all();
                        return;
                    }
                    q = self.wake.wait(q).unwrap();
                }
            };
            let mut kids = vec![];
            let res = scan::read_dir(&dir, &mut buf, &mut |e: scan::Ent| {
                let name = String::from_utf8_lossy(e.name).into_owned();
                if !self.hidden && name.starts_with('.') {
                    return;
                }
                let path = dir.join(scan::bytes_to_os(e.name));
                if e.dir {
                    if e.dev != 0 && self.root_dev != 0 && e.dev != self.root_dev {
                        self.skipped.fetch_add(1, Relaxed); // a mount point: another disk
                        return;
                    }
                    if self.m.matches(&name) {
                        self.hit(&path, name, true, 0);
                    }
                    if (self.enter)(&path) {
                        kids.push(path);
                    } else {
                        self.skipped.fetch_add(1, Relaxed);
                    }
                } else {
                    self.files.fetch_add(1, Relaxed);
                    if self.m.matches(&name) {
                        self.hit(&path, name, false, e.size);
                    }
                }
            });
            if res.is_err() {
                self.skipped.fetch_add(1, Relaxed);
            }
            self.dirs.fetch_add(1, Relaxed);
            let mut q = self.queue.lock().unwrap();
            q.0.extend(kids);
            q.1 -= 1;
            self.wake.notify_all();
        }
    }

    fn event(&self, id: &str, done: bool) -> SearchEvent {
        SearchEvent {
            id: id.to_string(),
            hits: std::mem::take(&mut *self.out.lock().unwrap()),
            done,
            dirs: self.dirs.load(Relaxed),
            files: self.files.load(Relaxed),
            skipped: self.skipped.load(Relaxed),
            capped: self.hits.load(Relaxed) >= MAX_HITS,
            error: None,
        }
    }
}

/// Walks `root` with several threads; `emit` gets a batch every [`FLUSH`] and a last one (`done`).
pub fn walk_local(
    id: &str,
    root: &Path,
    m: &Matcher,
    hidden: bool,
    enter: &(dyn Fn(&Path) -> bool + Sync),
    cancel: &AtomicBool,
    emit: &(dyn Fn(SearchEvent) + Sync),
) {
    let w = Walk {
        m,
        hidden,
        enter,
        cancel,
        root_dev: scan::dev_of(root).unwrap_or(0),
        queue: Mutex::new((VecDeque::from([root.to_path_buf()]), 0)),
        wake: Condvar::new(),
        out: Mutex::new(vec![]),
        hits: AtomicUsize::new(0),
        dirs: AtomicU64::new(0),
        files: AtomicU64::new(0),
        skipped: AtomicU64::new(0),
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let finished = AtomicBool::new(false);
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads).map(|_| s.spawn(|| w.worker())).collect();
        s.spawn(|| {
            while !finished.load(Acquire) {
                std::thread::sleep(FLUSH);
                let ev = w.event(id, false);
                if !ev.hits.is_empty() || !finished.load(Acquire) {
                    emit(ev);
                }
            }
        });
        for t in workers {
            let _ = t.join();
        }
        finished.store(true, Release);
    });
    emit(w.event(id, true));
}

fn register(id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    RUNNING.lock().unwrap().insert(id.to_string(), flag.clone());
    flag
}

fn unregister(id: &str) {
    RUNNING.lock().unwrap().remove(id);
}

/// Searches below the local folder `path`; results arrive as `"search"` events. Returns the id.
#[tauri::command]
pub fn search_local(app: AppHandle, path: String, query: String, hidden: bool) -> Result<String, String> {
    let m = Matcher::new(&query).ok_or("Type something to search for")?;
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err("That folder doesn't exist any more".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let cancel = register(&id);
    // Same rule as automatic size scans: no private folder the user hasn't opened (no prompts).
    let enter = app.state::<crate::sizes::SizeIndex>().enter_policy(&root, true);
    let sid = id.clone();
    std::thread::spawn(move || {
        let emit = |ev: SearchEvent| {
            let _ = app.emit("search", ev);
        };
        walk_local(&sid, &root, &m, hidden, &enter, &cancel, &emit);
        unregister(&sid);
    });
    Ok(id)
}

// ---------- remotes ----------

/// Matching files below `prefix`, plus the folders their paths go through whose name matches.
pub fn remote_hits(prefix: &str, files: Vec<Entry>, m: &Matcher) -> (Vec<Entry>, u64) {
    let mut dirs: HashSet<String> = HashSet::new();
    let mut hits = vec![];
    for f in files {
        let rel = f.key.strip_prefix(prefix).unwrap_or(&f.key);
        let mut at = prefix.to_string();
        let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        for seg in segs.iter().take(segs.len().saturating_sub(1)) {
            at.push_str(seg);
            at.push('/');
            if dirs.insert(at.clone()) && m.matches(seg) {
                hits.push(Entry {
                    key: at.clone(),
                    name: seg.to_string(),
                    size: 0,
                    modified: None,
                    dir: true,
                    hash: None,
                });
            }
        }
        if m.matches(&f.name) {
            hits.push(f);
        }
    }
    (hits, dirs.len() as u64)
}

/// Searches below `prefix` of a remote container; results arrive as `"search"` events.
#[tauri::command]
pub async fn search_remote(
    app: AppHandle,
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    prefix: String,
    query: String,
) -> Result<String, String> {
    let m = Matcher::new(&query).ok_or("Type something to search for")?;
    let remote = state.remote(account.as_deref())?;
    let id = uuid::Uuid::new_v4().to_string();
    let cancel = register(&id);
    let sid = id.clone();
    tauri::async_runtime::spawn(async move {
        let mut ev = SearchEvent { id: sid.clone(), done: true, ..Default::default() };
        match remote.list_files(&bucket, &prefix).await {
            Ok(files) => {
                ev.files = files.len() as u64;
                let (mut hits, dirs) = remote_hits(&prefix, files, &m);
                ev.dirs = dirs;
                ev.capped = hits.len() > MAX_HITS;
                hits.truncate(MAX_HITS);
                ev.hits = hits;
            }
            Err(e) => ev.error = Some(e),
        }
        if !cancel.load(Acquire) {
            let _ = app.emit("search", ev);
        }
        unregister(&sid);
    });
    Ok(id)
}

#[tauri::command]
pub fn cancel_search(id: String) {
    if let Some(f) = RUNNING.lock().unwrap().get(&id) {
        f.store(true, Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries() {
        let m = Matcher::new("report 2026").unwrap();
        assert!(m.matches("Annual-REPORT-2026.pdf"));
        assert!(!m.matches("report-2025.pdf"));
        let g = Matcher::new("IMG_????.jpg").unwrap();
        assert!(g.matches("img_0412.JPG") && !g.matches("img_04120.jpg") && !g.matches("x_img_0412.jpg"));
        let s = Matcher::new("*.tar.*").unwrap();
        assert!(s.matches("site.tar.gz") && !s.matches("site.zip"));
        let e = Matcher::new("ext:PDF").unwrap();
        assert!(e.matches("a.b.pdf") && !e.matches("pdf.txt"));
        assert!(Matcher::new("   ").is_none());
        assert!(glob(&['*'], &[]) && !glob(&['a'], &[]));
    }

    #[test]
    fn walks_in_parallel_skips_hidden_and_streams() {
        let dir = std::env::temp_dir().join(format!("noexplorer-ut-{}", uuid::Uuid::new_v4()));
        for i in 0..40 {
            let d = dir.join(format!("d{i}/inner{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("report-{i}.pdf")), b"x").unwrap();
            std::fs::write(d.join("notes.txt"), b"y").unwrap();
        }
        std::fs::create_dir_all(dir.join(".hidden/report-dir")).unwrap();
        std::fs::write(dir.join(".hidden/report-x.pdf"), b"z").unwrap();
        std::fs::create_dir_all(dir.join("private/report-in-private")).unwrap();
        let m = Matcher::new("report").unwrap();
        let events = Mutex::new(vec![]);
        let cancel = AtomicBool::new(false);
        let private = dir.join("private");
        let enter = move |p: &Path| p != private;
        let push = |ev: SearchEvent| events.lock().unwrap().push(ev);
        walk_local("t", &dir, &m, false, &enter, &cancel, &push);
        let events = std::mem::take(&mut *events.lock().unwrap());
        let last = events.last().unwrap();
        assert!(last.done && !last.capped);
        let (files, skipped) = (last.files, last.skipped);
        let hits: Vec<Entry> = events.into_iter().flat_map(|e| e.hits).collect();
        assert_eq!(hits.iter().filter(|h| !h.dir).count(), 40, "every report-N.pdf, nothing hidden");
        assert!(hits.iter().all(|h| !h.key.contains(".hidden") && !h.key.contains("report-in-private")));
        assert!(hits.iter().all(|h| h.modified.is_some()));
        assert_eq!(files, 80);
        assert!(skipped >= 1, "the private folder counts as skipped");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remote_hits_include_folders_once() {
        let f = |k: &str| Entry {
            key: k.into(),
            name: k.rsplit('/').next().unwrap().into(),
            size: 1,
            modified: None,
            dir: false,
            hash: None,
        };
        let files = vec![f("p/photos/2026/a.jpg"), f("p/photos/2026/b.jpg"), f("p/docs/photos-list.txt")];
        let (hits, dirs) = remote_hits("p/", files, &Matcher::new("photos").unwrap());
        let keys: Vec<&str> = hits.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(keys, ["p/photos/", "p/docs/photos-list.txt"]);
        assert_eq!(dirs, 3); // photos/, photos/2026/, docs/
    }

    /// `NOEXPLORER_BENCH_DIR=/usr NOEXPLORER_BENCH_Q=lib cargo test bench_search -- --ignored --nocapture`
    /// Compare with `find <dir> -xdev -iname '*<q>*' | wc -l` (warm cache; run twice).
    #[test]
    #[ignore]
    fn bench_search() {
        let dir = PathBuf::from(std::env::var("NOEXPLORER_BENCH_DIR").expect("NOEXPLORER_BENCH_DIR"));
        let q = std::env::var("NOEXPLORER_BENCH_Q").unwrap_or_else(|_| "lib".into());
        let m = Matcher::new(&q).unwrap();
        let cancel = AtomicBool::new(false);
        let n = AtomicUsize::new(0);
        let first = Mutex::new(None);
        let t = std::time::Instant::now();
        let count = |ev: SearchEvent| {
            if !ev.hits.is_empty() {
                first.lock().unwrap().get_or_insert(t.elapsed());
            }
            n.fetch_add(ev.hits.len(), Relaxed);
        };
        walk_local("b", &dir, &m, true, &|_: &Path| true, &cancel, &count);
        eprintln!("{} hits in {:?} (first batch after {:?})", n.load(Relaxed), t.elapsed(), first.lock().unwrap());
    }
}
