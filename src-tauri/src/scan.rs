//! Parallel folder-size scanner (the engine behind `sizes.rs`, ADR-022).
//!
//! - Directory reads are the hot path. On macOS one `getattrlistbulk` call returns name, type,
//!   device, inode, link count and size for a whole batch of entries, instead of a `stat` per file
//!   as `du`/ncdu do. Elsewhere `read_dir` is used (Windows already gets sizes from the listing).
//! - Workers go depth-first on their own stack and hand half of their pending folders to the
//!   shared queue only while another worker is idle, so the lock is rarely touched.
//! - Totals bubble up lock-free: every folder counts its unfinished children in `pending`;
//!   the last one to finish adds the folder's totals to its parent and reports it.
//! - Apparent sizes (what the file rows show), hard links counted once, symlinks never followed,
//!   other devices (mount points) not entered. Unreadable or skipped folders mark their ancestors
//!   `partial` (the totals are a lower bound; the UI explains it in a tooltip).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::{Arc, Condvar, Mutex};

/// Size of a folder and everything below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Totals {
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    /// Folders below that were skipped or unreadable: the totals are a lower bound.
    pub skipped: u32,
}

/// Receives each folder's totals as soon as its whole subtree is done (from worker threads).
pub trait Sink: Sync {
    fn done(&self, path: &Path, totals: Totals);
    /// A cloud-synced folder that wasn't entered (`Options::skip_cloud`).
    fn cloud(&self, _path: &Path, _service: &'static str) {}
}

pub struct Options<'a> {
    /// Called for every subfolder before it is entered; false = don't enter, count as skipped.
    pub enter: &'a (dyn Fn(&Path) -> bool + Sync),
    /// Totals already known (e.g. a folder that is indexed and watched): used instead of descending.
    pub known: &'a (dyn Fn(&Path) -> Option<Totals> + Sync),
    pub cancel: &'a AtomicBool,
    pub threads: usize,
    /// Don't enter online-only cloud folders (cloud.rs); they're reported to `Sink::cloud` instead.
    pub skip_cloud: bool,
    /// Where cloud folders live (None = this computer's; tests pass their own).
    pub places: Option<&'a crate::cloud::Places>,
}

struct Node {
    path: PathBuf,
    parent: Option<Arc<Node>>,
    bytes: AtomicU64,
    files: AtomicU64,
    dirs: AtomicU64,
    skipped: AtomicU32,
    /// Unfinished children + 1 for "not read yet".
    pending: AtomicU32,
}

impl Node {
    fn new(path: PathBuf, parent: Option<Arc<Node>>) -> Arc<Node> {
        Arc::new(Node {
            path,
            parent,
            bytes: AtomicU64::new(0),
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            skipped: AtomicU32::new(0),
            pending: AtomicU32::new(1),
        })
    }

    fn totals(&self) -> Totals {
        Totals {
            bytes: self.bytes.load(Acquire),
            files: self.files.load(Acquire),
            dirs: self.dirs.load(Acquire),
            skipped: self.skipped.load(Acquire),
        }
    }

    fn add(&self, t: Totals) {
        self.bytes.fetch_add(t.bytes, AcqRel);
        self.files.fetch_add(t.files, AcqRel);
        self.dirs.fetch_add(t.dirs, AcqRel);
        self.skipped.fetch_add(t.skipped, AcqRel);
    }
}

/// One entry of a directory listing.
pub struct Ent<'a> {
    pub name: &'a [u8],
    pub dir: bool,
    pub file: bool,
    /// Device id (unix), to stay on one filesystem. 0 when unknown.
    pub dev: u64,
    pub ino: u64,
    pub nlink: u32,
    pub size: u64,
}

struct Shared<'a> {
    queue: Mutex<Queue>,
    wake: Condvar,
    idle: AtomicUsize,
    links: Mutex<HashSet<(u64, u64)>>,
    root_dev: u64,
    opts: &'a Options<'a>,
    sink: &'a dyn Sink,
    places: std::borrow::Cow<'a, crate::cloud::Places>,
}

struct Queue {
    items: Vec<Arc<Node>>,
    /// Workers holding work (outside the queue).
    busy: usize,
}

/// Scans `root` and returns its totals (None if cancelled or unreadable).
pub fn scan(root: &Path, opts: &Options, sink: &dyn Sink) -> Option<Totals> {
    let root_dev = dev_of(root)?;
    let node = Node::new(root.to_path_buf(), None);
    let shared = Shared {
        queue: Mutex::new(Queue { items: vec![node.clone()], busy: 0 }),
        wake: Condvar::new(),
        idle: AtomicUsize::new(0),
        links: Mutex::new(HashSet::new()),
        root_dev,
        opts,
        sink,
        places: match opts.places {
            Some(p) => std::borrow::Cow::Borrowed(p),
            None => std::borrow::Cow::Owned(crate::cloud::Places::new()),
        },
    };
    std::thread::scope(|s| {
        for _ in 0..opts.threads.max(1) {
            s.spawn(|| worker(&shared));
        }
    });
    if opts.cancel.load(Relaxed) || node.pending.load(Acquire) != 0 {
        return None;
    }
    Some(node.totals())
}

fn worker(sh: &Shared) {
    let mut buf = vec![0u8; 256 * 1024];
    let mut stack: Vec<Arc<Node>> = Vec::with_capacity(256);
    loop {
        // Take work from the queue, or finish when nobody has any left.
        let first = {
            let mut q = sh.queue.lock().unwrap();
            loop {
                if sh.opts.cancel.load(Relaxed) {
                    sh.wake.notify_all();
                    return;
                }
                if let Some(n) = q.items.pop() {
                    q.busy += 1;
                    break n;
                }
                if q.busy == 0 {
                    sh.wake.notify_all();
                    return;
                }
                sh.idle.fetch_add(1, AcqRel);
                q = sh.wake.wait(q).unwrap();
                sh.idle.fetch_sub(1, AcqRel);
            }
        };
        stack.push(first);
        while let Some(node) = stack.pop() {
            if sh.opts.cancel.load(Relaxed) {
                break;
            }
            visit(sh, &node, &mut buf, &mut stack);
            // Someone is idle: share half of what we just found.
            if sh.idle.load(Acquire) > 0 && stack.len() > 1 {
                let give = stack.split_off(stack.len() / 2);
                let mut q = sh.queue.lock().unwrap();
                q.items.extend(give);
                sh.wake.notify_all();
            }
        }
        stack.clear();
        let mut q = sh.queue.lock().unwrap();
        q.busy -= 1;
        if q.busy == 0 && q.items.is_empty() {
            sh.wake.notify_all();
        }
    }
}

/// Reads one folder: adds its files, queues its subfolders, then releases it.
fn visit(sh: &Shared, node: &Arc<Node>, buf: &mut [u8], stack: &mut Vec<Arc<Node>>) {
    let mut own = Totals::default();
    let mut kids: Vec<PathBuf> = vec![];
    let res = read_dir(&node.path, buf, &mut |e: Ent| {
        if e.dir {
            if e.dev != 0 && sh.root_dev != 0 && e.dev != sh.root_dev {
                return; // a mount point: another filesystem
            }
            kids.push(node.path.join(bytes_to_os(e.name)));
        } else if e.file {
            if e.nlink > 1 && !sh.links.lock().unwrap().insert((e.dev, e.ino)) {
                return; // another name of a file we already counted
            }
            own.bytes += e.size;
            own.files += 1;
        }
    });
    if res.is_err() {
        own.skipped += 1;
    }
    let special = sh.opts.skip_cloud && sh.places.special(&node.path);
    let mut queued = 0u32;
    for path in kids {
        own.dirs += 1;
        if let Some(s) = special.then(|| path.file_name().and_then(|n| sh.places.at(&node.path, n))).flatten() {
            sh.sink.cloud(&path, s);
            own.skipped += 1;
        } else if let Some(t) = (sh.opts.known)(&path) {
            sh.sink.done(&path, t);
            own.add(t);
        } else if !(sh.opts.enter)(&path) {
            own.skipped += 1;
        } else {
            queued += 1;
            stack.push(Node::new(path, Some(node.clone())));
        }
    }
    node.pending.fetch_add(queued, AcqRel);
    node.add(own);
    release(sh, node);
}

/// One child (or the folder's own read) finished; completes folders up the tree as they finish.
fn release(sh: &Shared, node: &Arc<Node>) {
    let mut n = node.clone();
    loop {
        if n.pending.fetch_sub(1, AcqRel) != 1 {
            return;
        }
        let t = n.totals();
        sh.sink.done(&n.path, t);
        let Some(p) = n.parent.clone() else { return };
        p.add(t);
        n = p;
    }
}

trait TotalsAdd {
    fn add(&mut self, t: Totals);
}

impl TotalsAdd for Totals {
    fn add(&mut self, t: Totals) {
        self.bytes += t.bytes;
        self.files += t.files;
        self.dirs += t.dirs;
        self.skipped += t.skipped;
    }
}

#[cfg(unix)]
pub(crate) fn bytes_to_os(b: &[u8]) -> &std::ffi::OsStr {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(b)
}

#[cfg(not(unix))]
pub(crate) fn bytes_to_os(b: &[u8]) -> std::ffi::OsString {
    // Non-unix listings hand names over as UTF-8 (see the generic read_dir).
    String::from_utf8_lossy(b).into_owned().into()
}

#[cfg(unix)]
pub fn dev_of(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().filter(|m| m.is_dir()).map(|m| m.dev())
}

#[cfg(not(unix))]
pub fn dev_of(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().filter(|m| m.is_dir()).map(|_| 0)
}

// ---------- directory reading ----------

/// macOS: `getattrlistbulk`, one syscall per batch of entries (no per-file `stat`).
#[cfg(target_os = "macos")]
pub fn read_dir(path: &Path, buf: &mut [u8], f: &mut dyn FnMut(Ent)) -> std::io::Result<()> {
    use std::io::Error;
    use std::os::unix::ffi::OsStrExt;

    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    const VREG: u32 = 1;
    const VDIR: u32 = 2;

    struct Fd(i32);
    impl Drop for Fd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }

    let c = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(Error::last_os_error());
    }
    let fd = Fd(fd);
    let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
    al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    al.commonattr = libc::ATTR_CMN_RETURNED_ATTRS
        | ATTR_CMN_ERROR
        | libc::ATTR_CMN_NAME
        | libc::ATTR_CMN_DEVID
        | libc::ATTR_CMN_OBJTYPE
        | libc::ATTR_CMN_FILEID;
    al.fileattr = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_DATALENGTH;

    let u32_at = |b: &[u8], i: usize| u32::from_ne_bytes(b[i..i + 4].try_into().unwrap());
    let u64_at = |b: &[u8], i: usize| u64::from_ne_bytes(b[i..i + 8].try_into().unwrap());
    loop {
        let n = unsafe {
            libc::getattrlistbulk(fd.0, (&mut al as *mut libc::attrlist).cast(), buf.as_mut_ptr().cast(), buf.len(), 0)
        };
        if n < 0 {
            return Err(Error::last_os_error());
        }
        if n == 0 {
            return Ok(());
        }
        let mut p = 0usize;
        for _ in 0..n {
            let base = p;
            let len = u32_at(buf, p) as usize;
            // Packed in attribute-bit order: length, returned set, error, name, devid, objtype, fileid,
            // then the file attributes (only returned for files).
            let common = u32_at(buf, p + 4);
            let fileattr = u32_at(buf, p + 16);
            p += 4 + 20;
            if common & ATTR_CMN_ERROR != 0 {
                let err = u32_at(buf, p);
                p += 4;
                if err != 0 {
                    p = base + len;
                    continue;
                }
            }
            let name_ref = p;
            let off = i32::from_ne_bytes(buf[p..p + 4].try_into().unwrap()) as isize;
            let nlen = u32_at(buf, p + 4) as usize;
            p += 8;
            let start = (name_ref as isize + off) as usize;
            let name = &buf[start..start + nlen.saturating_sub(1)]; // drop the trailing NUL
            let dev = if common & libc::ATTR_CMN_DEVID != 0 {
                let d = u32_at(buf, p);
                p += 4;
                d as u64
            } else {
                0
            };
            let kind = if common & libc::ATTR_CMN_OBJTYPE != 0 {
                let k = u32_at(buf, p);
                p += 4;
                k
            } else {
                0
            };
            let ino = if common & libc::ATTR_CMN_FILEID != 0 {
                let i = u64_at(buf, p);
                p += 8;
                i
            } else {
                0
            };
            let (mut nlink, mut size) = (1, 0);
            if kind == VREG {
                if fileattr & libc::ATTR_FILE_LINKCOUNT != 0 {
                    nlink = u32_at(buf, p);
                    p += 4;
                }
                if fileattr & libc::ATTR_FILE_DATALENGTH != 0 {
                    size = u64_at(buf, p);
                }
            }
            f(Ent { name, dir: kind == VDIR, file: kind == VREG, dev, ino, nlink, size });
            p = base + len;
        }
    }
}

/// Everywhere else: `read_dir` (Linux: `d_type` + one `fstatat` per file; Windows: sizes come
/// with the listing).
#[cfg(not(target_os = "macos"))]
pub fn read_dir(path: &Path, _buf: &mut [u8], f: &mut dyn FnMut(Ent)) -> std::io::Result<()> {
    read_dir_generic(path, f)
}

#[allow(dead_code)] // macOS uses it only in tests (to check the fast path against it)
pub fn read_dir_generic(path: &Path, f: &mut dyn FnMut(Ent)) -> std::io::Result<()> {
    for e in std::fs::read_dir(path)? {
        let Ok(e) = e else { continue };
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name();
        #[cfg(unix)]
        let name_bytes = {
            use std::os::unix::ffi::OsStrExt;
            name.as_bytes().to_vec()
        };
        #[cfg(not(unix))]
        let name_bytes = name.to_string_lossy().into_owned().into_bytes();
        if ft.is_dir() {
            #[cfg(unix)]
            let dev = {
                use std::os::unix::fs::MetadataExt;
                e.metadata().map(|m| m.dev()).unwrap_or(0)
            };
            #[cfg(not(unix))]
            let dev = 0;
            f(Ent { name: &name_bytes, dir: true, file: false, dev, ino: 0, nlink: 1, size: 0 });
        } else if ft.is_file() {
            let Ok(md) = e.metadata() else { continue };
            #[cfg(unix)]
            let (dev, ino, nlink) = {
                use std::os::unix::fs::MetadataExt;
                (md.dev(), md.ino(), md.nlink() as u32)
            };
            #[cfg(not(unix))]
            let (dev, ino, nlink) = (0, 0, 1);
            f(Ent { name: &name_bytes, dir: false, file: true, dev, ino, nlink, size: md.len() });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Instant;

    struct Collect(Mutex<HashMap<PathBuf, Totals>>);
    impl Sink for Collect {
        fn done(&self, path: &Path, t: Totals) {
            self.0.lock().unwrap().insert(path.to_path_buf(), t);
        }
    }

    fn opts<'a>(cancel: &'a AtomicBool, enter: &'a (dyn Fn(&Path) -> bool + Sync)) -> Options<'a> {
        Options { enter, known: &|_| None, cancel, threads: 4, skip_cloud: false, places: None }
    }

    fn tree() -> PathBuf {
        let d = std::env::temp_dir().join(format!("nx-scan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join("a/b/c")).unwrap();
        std::fs::create_dir_all(d.join("skip/me")).unwrap();
        std::fs::create_dir_all(d.join("empty")).unwrap();
        std::fs::write(d.join("top.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(d.join("a/one.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(d.join("a/b/two.bin"), vec![0u8; 20]).unwrap();
        std::fs::write(d.join("a/b/c/three.bin"), vec![0u8; 3]).unwrap();
        std::fs::write(d.join("skip/me/big.bin"), vec![0u8; 5000]).unwrap();
        #[cfg(unix)]
        {
            std::fs::hard_link(d.join("a/one.bin"), d.join("a/b/one-again.bin")).unwrap();
            std::os::unix::fs::symlink(d.join("skip"), d.join("a/link-to-skip")).unwrap();
        }
        d
    }

    /// An online-only cloud folder (by location) isn't entered: reported as cloud, counted as skipped.
    /// A sync client's folder of real files (Nextcloud's marker, no virtual files) is counted.
    #[test]
    fn cloud_folders_are_reported_not_counted() {
        struct Clouds(Mutex<Vec<(PathBuf, &'static str)>>);
        impl Sink for Clouds {
            fn done(&self, _: &Path, _: Totals) {}
            fn cloud(&self, p: &Path, s: &'static str) {
                self.0.lock().unwrap().push((p.to_path_buf(), s));
            }
        }
        let d = tree();
        let od = d.join("Library/CloudStorage/OneDrive-Personal");
        std::fs::create_dir_all(&od).unwrap();
        std::fs::write(od.join("doc.bin"), vec![0u8; 9000]).unwrap();
        std::fs::create_dir_all(d.join("nc.example.com")).unwrap();
        std::fs::write(d.join("nc.example.com/.sync_abc123.db"), vec![0u8; 50]).unwrap();
        let places = crate::cloud::Places::rooted(&d);
        let cancel = AtomicBool::new(false);
        let enter = |p: &Path| !p.ends_with("skip");
        let sink = Clouds(Mutex::new(vec![]));
        let o = Options { skip_cloud: true, places: Some(&places), ..opts(&cancel, &enter) };
        let t = scan(&d, &o, &sink).unwrap();
        assert_eq!((t.bytes, t.skipped), (1123 + 50, 2), "OneDrive adds nothing; the Nextcloud folder counts");
        assert_eq!(*sink.0.lock().unwrap(), [(od.clone(), "OneDrive")]);
        // Σ inside it (the scan's own root) counts it.
        assert_eq!(scan(&od, &o, &sink).unwrap().bytes, 9000);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn sums_subtrees_counts_hardlinks_once_and_skips() {
        let d = tree();
        let cancel = AtomicBool::new(false);
        let skip = d.join("skip");
        let enter = |p: &Path| p != skip;
        let sink = Collect(Mutex::new(HashMap::new()));
        let t = scan(&d, &opts(&cancel, &enter), &sink).unwrap();
        // 1000 + 100 + 20 + 3; the hard link and the symlinked folder add nothing; skip/ is skipped.
        assert_eq!((t.bytes, t.files, t.skipped), (1123, 4, 1));
        let m = sink.0.lock().unwrap();
        assert_eq!(m[&d.join("a")].bytes, 123);
        assert_eq!(m[&d.join("a/b")].bytes, 23);
        assert_eq!(m[&d.join("a/b/c")], Totals { bytes: 3, files: 1, dirs: 0, skipped: 0 });
        assert_eq!(m[&d.join("empty")], Totals::default());
        assert!(!m.contains_key(&d.join("skip")));
        let all = scan(&d, &opts(&cancel, &|_| true), &Collect(Mutex::new(HashMap::new()))).unwrap();
        assert_eq!((all.bytes, all.files, all.skipped), (6123, 5, 0));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn known_totals_replace_descending() {
        let d = tree();
        let cancel = AtomicBool::new(false);
        let a = d.join("a");
        let known = move |p: &Path| (p == a).then_some(Totals { bytes: 7, files: 1, dirs: 0, skipped: 0 });
        let o =
            Options { enter: &|_| true, known: &known, cancel: &cancel, threads: 2, skip_cloud: false, places: None };
        let t = scan(&d, &o, &Collect(Mutex::new(HashMap::new()))).unwrap();
        assert_eq!(t.bytes, 1000 + 7 + 5000);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The macOS fast path must agree with plain `read_dir` + `stat`.
    #[test]
    fn fast_listing_matches_generic() {
        let d = tree();
        let mut buf = vec![0u8; 4096]; // small buffer: forces several getattrlistbulk calls
        let mut fast = vec![];
        read_dir(&d.join("a"), &mut buf, &mut |e| {
            fast.push((String::from_utf8_lossy(e.name).into_owned(), e.dir, e.file, e.size, e.nlink))
        })
        .unwrap();
        let mut slow = vec![];
        read_dir_generic(&d.join("a"), &mut |e| {
            slow.push((String::from_utf8_lossy(e.name).into_owned(), e.dir, e.file, e.size, e.nlink))
        })
        .unwrap();
        fast.retain(|e| e.1 || e.2); // symlinks are reported but never counted
        fast.sort();
        slow.sort();
        assert_eq!(fast, slow);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// `NOEXPLORER_BENCH_DIR=~/some/big/folder cargo test --release bench_scan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_scan() {
        let dir = PathBuf::from(std::env::var("NOEXPLORER_BENCH_DIR").expect("set NOEXPLORER_BENCH_DIR"));
        for threads in [4, 1, 1, 2, 2, 4, 4, 6, 6, 8, 8, 12, 12] {
            let cancel = AtomicBool::new(false);
            let o = Options {
                enter: &|_| true,
                known: &|_| None,
                cancel: &cancel,
                threads,
                skip_cloud: false,
                places: None,
            };
            let sink = Collect(Mutex::new(HashMap::new()));
            let t0 = Instant::now();
            let t = scan(&dir, &o, &sink).unwrap();
            println!(
                "threads={threads:2}  {:>7.0} ms  {} files  {} dirs  {} bytes",
                t0.elapsed().as_secs_f64() * 1000.0,
                t.files,
                t.dirs,
                t.bytes
            );
        }
    }
}
