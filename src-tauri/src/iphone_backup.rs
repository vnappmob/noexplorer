//! iPhone / iPad backups (experimental, ADR-055): NoExplorer asks the device for a backup over
//! mobilebackup2, as Finder / Apple Devices do, keeps it in its own data folder, and shows the apps'
//! files from it, read-only, as extra containers of the iPhone account ("Backup · WhatsApp").
//!
//! - A backup is the whole device (iOS has no per-app backup); later ones only bring what changed.
//! - The device writes each file under its SHA-1 name (`ab/ab12…`) and lists them in `Manifest.db`
//!   (SQLite): domain (`AppDomain-<bundle id>`, `AppDomainGroup-…`, `HomeDomain`…), path, kind and
//!   an archived `MBFile` with size and date. [`Index`] turns that into folders.
//! - Encrypted backups ("Encrypt local backup" on) aren't readable yet: the backup is refused
//!   before it starts, and an encrypted one found on disk says so.
//! - Apps' display names come from the device when the backup is made (`noexplorer-apps.json`), so
//!   a backup can be browsed with the phone unplugged.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use idevice::afc::AfcClient;
use idevice::afc::errors::AfcError;
use idevice::afc::opcode::{AfcFopenMode, AfcLockOp};
use idevice::lockdown::LockdownClient;
use idevice::mobilebackup2::{BackupDelegate, BackupProgress, DirEntryInfo, FsBackupDelegate, MobileBackup2Client};
use idevice::provider::IdeviceProvider;
use idevice::{IdeviceError, IdeviceService};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::iphone::IPhone;
use crate::jobs::Job;
use crate::provider::{Entry, Remote};
use crate::state::AppState;

/// Containers read from the backup start with this.
pub const PREFIX: &str = "Backup · ";
/// The container for everything that belongs to no app (HomeDomain, MediaDomain…).
const OTHER: &str = "Other";
pub const READ_ONLY: &str = "A backup is read-only. Back up again to see newer files.";
const ENCRYPTED: &str = "This iPhone encrypts its backups (“Encrypt local backup” is on in Finder or Apple Devices). NoExplorer can't read encrypted backups yet: turn that off there (it asks for the backup password), then back up again.";
const APPS_FILE: &str = "noexplorer-apps.json";
/// The device as the Devices group showed it at backup time (name, model).
const DEVICE_FILE: &str = "noexplorer-device.json";
/// In a backup folder while a backup runs, and left there when it doesn't finish.
const INCOMPLETE: &str = "noexplorer-incomplete";
/// The device sending nothing for this long (while still connected) ends the backup. Generous:
/// iOS may first wait for its passcode, and prepares big backups quietly.
const QUIET_LIMIT: Duration = Duration::from_secs(10 * 60);
/// Free space needed to start, and below which a running backup stops (the disk is the user's).
const START_FREE: u64 = 2_000_000_000;
const KEEP_FREE: u64 = 1_000_000_000;
pub const RUNNING: &str = "Backing up… The apps' files show here once the backup finishes.";
pub const UNFINISHED: &str = "The last backup didn't finish, so there's nothing to read yet: iOS only keeps a backup once it finishes. Connect the iPhone and back up again, or delete the backup (right-click) to free the space it took.";

static ROOT: OnceLock<PathBuf> = OnceLock::new();
/// Devices (UDIDs) being backed up right now.
static BUSY: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

pub fn init(app: &AppHandle) {
    if let Ok(d) = app.path().app_data_dir() {
        let _ = ROOT.set(d.join("iphone-backups"));
    }
}

/// Where backups live: one folder per device (its UDID), as Finder lays them out.
pub fn root() -> Result<PathBuf, String> {
    ROOT.get().cloned().ok_or_else(|| "There's no folder for backups".into())
}

pub fn is_backup(container: &str) -> bool {
    container.starts_with(PREFIX)
}

/// A backup was started here and never finished (it may have left many GB in `Snapshot/`).
pub fn unfinished(dir: &Path) -> bool {
    dir.join(INCOMPLETE).exists()
}

pub fn busy(udid: &str) -> bool {
    BUSY.lock().unwrap().contains(udid)
}

/// The most recent backup on this computer, for an account that takes any device.
pub fn latest() -> Option<String> {
    let dirs = std::fs::read_dir(root().ok()?).ok()?;
    dirs.flatten()
        .filter_map(|e| {
            let t = e.path().join("Manifest.db").metadata().and_then(|m| m.modified()).ok()?;
            Some((t, e.file_name().to_string_lossy().to_string()))
        })
        .max()
        .map(|(_, udid)| udid)
}

/// Devices with a backup here (for the Devices group, connected or not): name and model from
/// what we saved at backup time, else from the device's own `Manifest.plist`.
pub fn saved_devices() -> Vec<crate::devices::DeviceInfo> {
    let Ok(dirs) = root().and_then(|r| std::fs::read_dir(r).map_err(|e| e.to_string())) else { return vec![] };
    let mut out = vec![];
    for e in dirs.flatten() {
        let dir = e.path();
        let Ok(t) = dir.join("Manifest.db").metadata().and_then(|m| m.modified()) else { continue };
        let udid = e.file_name().to_string_lossy().to_string();
        let lockdown = || -> Option<plist::Dictionary> {
            let m: plist::Dictionary = plist::from_file(dir.join("Manifest.plist")).ok()?;
            m.get("Lockdown")?.as_dictionary().cloned()
        };
        let saved: Option<crate::devices::DeviceInfo> =
            std::fs::read(dir.join(DEVICE_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let mut d = saved.unwrap_or_else(|| {
            let l = lockdown().unwrap_or_default();
            let s = |k: &str| l.get(k).and_then(|v| v.as_string()).unwrap_or_default().to_string();
            crate::devices::DeviceInfo {
                udid: udid.clone(),
                name: Some(s("DeviceName")).filter(|n| !n.is_empty()).unwrap_or_else(|| "iPhone".into()),
                class: "iPhone".into(),
                model: s("ProductType"),
                usb: false,
                trusted: true,
                connected: false,
                backup: None,
            }
        });
        d.udid = udid;
        d.connected = false;
        d.backup = t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as i64);
        out.push(d);
    }
    out
}

// ---------- reading a backup ----------

#[derive(Clone, Debug)]
struct Node {
    key: String,
    name: String,
    dir: bool,
    size: u64,
    modified: Option<i64>,
}

#[derive(Default)]
struct Tree {
    /// Folder key ("" = top) → what's in it.
    children: HashMap<String, Vec<Node>>,
    /// File key → its name in the backup (SHA-1).
    files: HashMap<String, String>,
}

impl Tree {
    /// Adds `key`, and any folder above it that has no row of its own.
    fn add(&mut self, node: Node, file_id: Option<String>) {
        let parent = parent_of(&node.key);
        if !parent.is_empty() && !self.children.contains_key(&parent) {
            let name = parent.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
            self.add(Node { key: parent.clone(), name, dir: true, size: 0, modified: None }, None);
        }
        if node.dir {
            if self.children.contains_key(&node.key) {
                return; // already there (made for a child listed first)
            }
            self.children.insert(node.key.clone(), vec![]);
        }
        if let Some(id) = file_id {
            self.files.insert(node.key.clone(), id);
        }
        self.children.entry(parent).or_default().push(node);
    }
}

/// `a/b/` → `a/`, `a/b.txt` → `a/`, `a` → ``.
fn parent_of(key: &str) -> String {
    match key.trim_end_matches('/').rfind('/') {
        Some(i) => key[..=i].to_string(),
        None => String::new(),
    }
}

/// One row of `Manifest.db`'s `Files` table.
struct Row {
    file_id: String,
    domain: String,
    path: String,
    /// 1 = file, 2 = folder, 4 = symbolic link.
    flags: i64,
    meta: Option<Vec<u8>>,
}

/// A backup's files, by container.
pub struct Index {
    dir: PathBuf,
    trees: BTreeMap<String, Tree>,
}

impl Index {
    pub fn load(dir: &Path) -> Result<Index, String> {
        if encrypted(dir) {
            return Err(ENCRYPTED.into());
        }
        let rows = match read_manifest(&dir.join("Manifest.db")) {
            Err(_) if dir.join(INCOMPLETE).exists() => return Err(UNFINISHED.into()),
            r => r?,
        };
        let names: HashMap<String, String> =
            std::fs::read(dir.join(APPS_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Ok(Index { dir: dir.to_path_buf(), trees: build(rows, &names) })
    }

    pub fn containers(&self) -> Vec<String> {
        self.trees.keys().cloned().collect()
    }

    fn tree(&self, container: &str) -> Result<&Tree, String> {
        self.trees.get(container).ok_or_else(|| format!("“{container}” isn't in this iPhone's backup"))
    }

    pub fn list(&self, container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let t = self.tree(container)?;
        let nodes = t.children.get(prefix).ok_or("Not found")?;
        Ok(nodes
            .iter()
            .map(|n| Entry {
                key: n.key.clone(),
                name: n.name.clone(),
                size: n.size,
                modified: n.modified,
                dir: n.dir,
                hash: None,
            })
            .collect())
    }

    /// The backed-up copy of a file.
    pub fn file(&self, container: &str, key: &str) -> Result<PathBuf, String> {
        let id = self.tree(container)?.files.get(key).ok_or("Not found")?;
        Ok(self.dir.join(id.get(..2).unwrap_or_default()).join(id))
    }
}

fn encrypted(dir: &Path) -> bool {
    plist::from_file::<_, plist::Dictionary>(dir.join("Manifest.plist"))
        .ok()
        .and_then(|d| d.get("IsEncrypted").and_then(|v| v.as_boolean()))
        .unwrap_or(false)
}

fn read_manifest(db: &Path) -> Result<Vec<Row>, String> {
    use rusqlite::{Connection, OpenFlags};
    let err = |e: rusqlite::Error| format!("Couldn't read the backup's file list: {e}");
    let c = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(err)?;
    // Sorted, so a folder's row comes before what's in it.
    let mut q = c
        .prepare("SELECT fileID, domain, relativePath, flags, file FROM Files ORDER BY domain, relativePath")
        .map_err(err)?;
    let rows = q
        .query_map([], |r| {
            Ok(Row { file_id: r.get(0)?, domain: r.get(1)?, path: r.get(2)?, flags: r.get(3)?, meta: r.get(4)? })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

/// Size and modification time (Unix seconds) from a row's archived `MBFile` (NSKeyedArchiver).
fn file_meta(blob: &[u8]) -> (Option<u64>, Option<i64>) {
    let Ok(plist::Value::Dictionary(d)) = plist::from_bytes::<plist::Value>(blob) else { return (None, None) };
    let objects = d.get("$objects").and_then(|o| o.as_array());
    let root = d.get("$top").and_then(|t| t.as_dictionary()).and_then(|t| t.get("root"));
    let obj = match (objects, root) {
        (Some(o), Some(plist::Value::Uid(u))) => o.get(u.get() as usize).and_then(|v| v.as_dictionary()),
        _ => None,
    };
    let Some(obj) = obj else { return (None, None) };
    let int = |k: &str| obj.get(k).and_then(|v| v.as_signed_integer().or(v.as_unsigned_integer().map(|u| u as i64)));
    (int("Size").map(|s| s.max(0) as u64), int("LastModified"))
}

/// Where a domain's files go: the app that owns it (bundle id, None = "Other") and the folder
/// they're under there.
fn place(domain: &str, bundles: &[&str]) -> (Option<String>, String) {
    // The longest bundle id the group or extension belongs to (group.net.whatsapp.WhatsApp.shared
    // → net.whatsapp.WhatsApp; net.whatsapp.WhatsApp.Share → net.whatsapp.WhatsApp).
    let owner = |id: &str, contains: bool| {
        bundles
            .iter()
            .filter(|b| if contains { id.contains(**b) } else { id.starts_with(&format!("{b}.")) })
            .max_by_key(|b| b.len())
            .map(|b| b.to_string())
    };
    if let Some(b) = domain.strip_prefix("AppDomain-") {
        return (Some(b.to_string()), String::new());
    }
    let (found, folder) = if let Some(g) = domain.strip_prefix("AppDomainGroup-") {
        (owner(g, true), format!("App Group · {g}/"))
    } else if let Some(p) = domain.strip_prefix("AppDomainPlugin-") {
        (owner(p, false), format!("Extension · {p}/"))
    } else {
        (None, String::new())
    };
    match found {
        Some(b) => (Some(b), folder),
        None => (None, format!("{domain}/")),
    }
}

/// Container names: `Backup · <app name>`, with the bundle id when two apps share a name (or one
/// is called "Other").
fn container_names(bundles: &[&str], names: &HashMap<String, String>) -> HashMap<String, String> {
    let shown = |b: &str| names.get(b).map(|n| n.trim()).filter(|n| !n.is_empty()).unwrap_or(b).to_string();
    let mut count: HashMap<String, usize> = HashMap::from([(OTHER.to_lowercase(), 1)]);
    for b in bundles {
        *count.entry(shown(b).to_lowercase()).or_default() += 1;
    }
    bundles
        .iter()
        .map(|b| {
            let n = shown(b).replace('/', "∕"); // a folder name
            let n = if count[&n.to_lowercase()] > 1 && n != *b { format!("{n} ({b})") } else { n };
            (b.to_string(), format!("{PREFIX}{n}"))
        })
        .collect()
}

fn build(rows: Vec<Row>, names: &HashMap<String, String>) -> BTreeMap<String, Tree> {
    let bundles: Vec<&str> = {
        let mut b: Vec<&str> = rows.iter().filter_map(|r| r.domain.strip_prefix("AppDomain-")).collect();
        b.dedup();
        b
    };
    let containers = container_names(&bundles, names);
    let other = format!("{PREFIX}{OTHER}");
    let mut trees: BTreeMap<String, Tree> = BTreeMap::new();
    let mut placed: HashMap<&str, (String, String)> = HashMap::new();
    for r in &rows {
        if r.path.is_empty() || r.flags == 4 {
            continue; // the domain itself, or a link
        }
        let (container, folder) = placed
            .entry(r.domain.as_str())
            .or_insert_with(|| {
                let (owner, folder) = place(&r.domain, &bundles);
                (owner.and_then(|b| containers.get(&b).cloned()).unwrap_or_else(|| other.clone()), folder)
            })
            .clone();
        let dir = r.flags == 2;
        let path: Vec<&str> = r.path.split('/').filter(|s| !s.is_empty() && *s != "." && *s != "..").collect();
        let Some(name) = path.last() else { continue };
        let (size, modified) = r.meta.as_deref().map(file_meta).unwrap_or_default();
        let node = Node {
            key: format!("{folder}{}{}", path.join("/"), if dir { "/" } else { "" }),
            name: name.to_string(),
            dir,
            size: if dir { 0 } else { size.unwrap_or(0) },
            modified: modified.map(|s| s * 1000),
        };
        trees.entry(container).or_default().add(node, (!dir).then(|| r.file_id.clone()));
    }
    trees
}

// ---------- making a backup ----------

/// The crate's file system delegate, plus progress for the job and the time the device last
/// asked for something (the watchdog's "is it still talking?").
struct Progress<'a> {
    job: &'a Job,
    last: Mutex<Instant>,
}

type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

impl Progress<'_> {
    fn touch(&self) {
        *self.last.lock().unwrap() = Instant::now();
    }
    fn quiet_for(&self) -> Duration {
        self.last.lock().unwrap().elapsed()
    }
}

impl BackupDelegate for Progress<'_> {
    fn get_free_disk_space(&self, path: &Path) -> u64 {
        self.touch();
        FsBackupDelegate.get_free_disk_space(path)
    }
    fn open_file_read<'a>(&'a self, path: &'a Path) -> Fut<'a, Result<Box<dyn Read + Send>, IdeviceError>> {
        self.touch();
        FsBackupDelegate.open_file_read(path)
    }
    fn create_file_write<'a>(&'a self, path: &'a Path) -> Fut<'a, Result<Box<dyn Write + Send>, IdeviceError>> {
        self.touch();
        FsBackupDelegate.create_file_write(path)
    }
    fn create_dir_all<'a>(&'a self, path: &'a Path) -> Fut<'a, Result<(), IdeviceError>> {
        self.touch();
        FsBackupDelegate.create_dir_all(path)
    }
    fn remove<'a>(&'a self, path: &'a Path) -> Fut<'a, Result<(), IdeviceError>> {
        self.touch();
        FsBackupDelegate.remove(path)
    }
    fn rename<'a>(&'a self, from: &'a Path, to: &'a Path) -> Fut<'a, Result<(), IdeviceError>> {
        self.touch();
        FsBackupDelegate.rename(from, to)
    }
    fn copy<'a>(&'a self, src: &'a Path, dst: &'a Path) -> Fut<'a, Result<(), IdeviceError>> {
        self.touch();
        FsBackupDelegate.copy(src, dst)
    }
    fn exists<'a>(&'a self, path: &'a Path) -> Fut<'a, bool> {
        self.touch();
        FsBackupDelegate.exists(path)
    }
    fn is_dir<'a>(&'a self, path: &'a Path) -> Fut<'a, bool> {
        self.touch();
        FsBackupDelegate.is_dir(path)
    }
    fn list_dir<'a>(&'a self, path: &'a Path) -> Fut<'a, Result<Vec<DirEntryInfo>, IdeviceError>> {
        self.touch();
        FsBackupDelegate.list_dir(path)
    }
    fn on_file_received(&self, _path: &str, _file_count: u32) {
        self.touch();
        self.job.files_done.fetch_add(1, Ordering::Relaxed);
    }
    fn on_progress(&self, p: BackupProgress) {
        self.touch();
        self.job.bytes_done.store(p.session_bytes_done, Ordering::Relaxed);
        // The device only says how far along it is; the total is estimated from that.
        if p.session_bytes_total > 0 {
            self.job.bytes_total.store(p.session_bytes_total.max(p.session_bytes_done), Ordering::Relaxed);
        }
    }
}

/// Leaves `BUSY` however the backup ends.
struct Busy(String);

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.lock().unwrap().remove(&self.0);
    }
}

/// mobilebackup2, started the way Finder and libimobiledevice start it: with the escrow keybag
/// from the pair record (`EscrowBag` in StartService). It lets the backup read files that iOS only
/// opens while the device is unlocked (data protection classes A/B: Mail, Health, many apps'
/// databases). Without it, those files fail as soon as the screen locks during a long backup, and
/// the device ends with MBErrorDomain 104 "Multiple errors uploading files". The crate's
/// `MobileBackup2Client::connect` doesn't present it.
async fn connect_backup(p: &dyn IdeviceProvider) -> Result<(MobileBackup2Client, bool), IdeviceError> {
    let pairing = p.get_pairing_file().await?;
    let escrow = pairing.escrow_bag.is_some();
    let mut l = LockdownClient::connect(p).await?;
    let legacy = l.start_session(&pairing).await?;
    let udid = l.get_value(Some("UniqueDeviceID"), None).await.ok().and_then(|v| v.as_string().map(str::to_string));
    let (port, ssl) =
        l.start_service_with_escrow(MobileBackup2Client::service_name(), pairing.escrow_bag.clone()).await?;
    let mut dev = p.connect(port).await?;
    if ssl {
        dev.start_session(&pairing, legacy).await?;
    }
    if let Some(u) = udid {
        dev.set_udid(u);
    }
    Ok((MobileBackup2Client::from_stream(dev).await?, escrow))
}

/// The device's own reason for a failed backup, with what to do about it. iOS keeps a backup only
/// once it finishes (Status.plist `SnapshotState` "finished"): until then everything sits in
/// `Snapshot/`, and the next backup starts over.
fn device_failure(code: i64, why: &str, escrow: bool) -> String {
    match code {
        // Some files couldn't be read on the device. With the escrow keybag that's rare; left:
        // files changing while being backed up, or a phone not unlocked since it restarted.
        104 if escrow => format!(
            "The iPhone couldn't read some of its files ({why}, {code}). Unlock it, keep it on its Home Screen with busy apps (a big download, a call) closed, and back up again. If it keeps happening, restart the iPhone, unlock it once, and back up again."
        ),
        104 => format!(
            "The iPhone couldn't read files it only opens while unlocked ({why}, {code}): this computer's pairing with it has no unlock key. Set Auto-Lock to Never (Settings → Display & Brightness) while backing up, then back up again."
        ),
        _ => format!("iPhone: {why} ({code})."),
    }
}

fn gb(n: u64) -> String {
    format!("{:.1} GB", n as f64 / 1e9)
}

/// Why a backup stopped, in the user's terms (the device's own reason when it gave one).
fn stopped(e: IdeviceError) -> String {
    match crate::iphone::device_err(e) {
        m if m.starts_with("iPhone: ") => m,
        m => format!("{m} The backup didn't finish: back up again to complete it."),
    }
}

/// Backs up the device behind `ph` into `root()/<udid>`. Every wait is bounded: the device being
/// unplugged, going quiet, or this computer's disk filling up ends it with a reason, and the
/// folder is marked unfinished (`INCOMPLETE`) until a backup succeeds.
async fn run(ph: &IPhone, job: &Job) -> Result<(), String> {
    use crate::iphone::{CONNECT_TIMEOUT, guarded, unplugged};
    let d = ph.device().await?;
    let (udid, id) = (d.udid.clone(), d.device_id);
    if !BUSY.lock().unwrap().insert(udid.clone()) {
        return Err("This iPhone is already being backed up".into());
    }
    let _busy = Busy(udid.clone());
    let root = root()?;
    let dir = root.join(&udid);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    *job.local_dir.lock().unwrap() = Some(dir.to_string_lossy().to_string());
    let free = FsBackupDelegate.get_free_disk_space(&dir);
    if free < START_FREE {
        return Err(format!(
            "This computer has only {} free. Make room (a first backup needs about as much as the data on the iPhone), then back up again.",
            gb(free)
        ));
    }
    let p = d.to_provider(crate::iphone::addr(), crate::iphone::LABEL);

    // Refuse an encrypted backup before it starts: we couldn't read it.
    let will_encrypt = guarded(id, CONNECT_TIMEOUT, async {
        let mut l = LockdownClient::connect(&p).await?;
        l.start_session(&p.get_pairing_file().await?).await?;
        l.get_value(Some("WillEncrypt"), Some("com.apple.mobile.backup")).await
    });
    match will_encrypt.await {
        Ok(v) if v.as_boolean() == Some(true) => return Err(ENCRYPTED.into()),
        Ok(_) => {}
        Err(e) => return Err(crate::iphone::explain(&d, e).await),
    }
    // Which device this is, for the Devices group when it isn't connected.
    if let Some(me) = crate::devices::list().into_iter().find(|x| x.udid == udid && x.connected) {
        let _ = tokio::fs::write(dir.join(DEVICE_FILE), serde_json::to_vec(&me).unwrap_or_default()).await;
    }
    // Apps' names, to browse the backup by app later (also with the phone unplugged).
    if let Ok(apps) = ph.app_names().await {
        let _ = tokio::fs::write(dir.join(APPS_FILE), serde_json::to_vec(&apps).unwrap_or_default()).await;
    }

    // The sync lock, as Finder and libimobiledevice take it: no other sync while we back up.
    let mut afc = guarded(id, CONNECT_TIMEOUT, AfcClient::connect(&p)).await.map_err(crate::iphone::device_err)?;
    let mut lock = guarded(id, CONNECT_TIMEOUT, afc.open("/com.apple.itunes.lock_sync", AfcFopenMode::Rw))
        .await
        .map_err(crate::iphone::device_err)?;
    let mut tries = 0;
    loop {
        match guarded(id, CONNECT_TIMEOUT, lock.lock(AfcLockOp::ExclusiveLock)).await {
            Ok(()) => break,
            Err(IdeviceError::Afc(AfcError::OpWouldBlock)) if tries < 50 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(IdeviceError::Afc(_)) => {
                return Err("Finder, Apple Devices or iTunes is syncing this iPhone. Try again when it's done.".into());
            }
            Err(e) => return Err(crate::iphone::device_err(e)),
        }
    }

    // From here until it succeeds, what's on disk may be half old, half new.
    let _ = tokio::fs::write(dir.join(INCOMPLETE), b"").await;
    let delegate = Progress { job, last: Mutex::new(Instant::now()) };
    let has_escrow = Mutex::new(true);
    let res = async {
        let (mut mb, escrow) = guarded(id, CONNECT_TIMEOUT, connect_backup(&p)).await.map_err(stopped)?;
        *has_escrow.lock().unwrap() = escrow;
        let backup = mb.backup_from_path(&root, Some(&udid), None, &delegate);
        // Once a second: cancelled, the device gone quiet, or the disk nearly full.
        let watch = async {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if job.cancelled() {
                    return "cancelled".to_string();
                }
                if delegate.quiet_for() > QUIET_LIMIT {
                    return format!(
                        "The iPhone sent nothing for {} minutes. If it asked for its passcode, enter it; otherwise unplug it and plug it back in. Then back up again.",
                        QUIET_LIMIT.as_secs() / 60
                    );
                }
                let free = FsBackupDelegate.get_free_disk_space(&dir);
                if free < KEEP_FREE {
                    return format!(
                        "Stopped: this computer's disk is almost full ({} left). Make room, then back up again.",
                        gb(free)
                    );
                }
            }
        };
        tokio::select! {
            r = backup => r.map_err(stopped),
            e = watch => Err(e),
            _ = unplugged(id) => Err("The iPhone was disconnected during the backup. Connect it again and back up again to complete it.".into()),
        }
    }
    .await;
    // Best effort, and never a wait: on a lost device these would hang or fail anyway.
    let _ = tokio::time::timeout(Duration::from_secs(3), lock.lock(AfcLockOp::Unlock)).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), lock.close()).await;
    ph.forget_backup();

    let done = res?;
    let code = done.as_ref().and_then(|d| d.get("ErrorCode")).and_then(|c| c.as_signed_integer()).unwrap_or(0);
    if code != 0 {
        let why = done
            .as_ref()
            .and_then(|d| d.get("ErrorDescription"))
            .and_then(|s| s.as_string())
            .unwrap_or("the backup failed")
            .to_string();
        let escrow = *has_escrow.lock().unwrap();
        return Err(format!(
            "{} iOS only keeps a backup once it finishes, so the next one starts over.",
            device_failure(code, &why, escrow)
        ));
    }
    let _ = tokio::fs::remove_file(dir.join(INCOMPLETE)).await;
    Ok(())
}

// ---------- commands ----------

fn iphone(r: &Remote) -> Result<&IPhone, String> {
    match r {
        Remote::IPhone(i) => Ok(i),
        _ => Err("This account isn't an iPhone".into()),
    }
}

/// Starts a backup of the account's iPhone. Returns the job id (kind "backup").
#[tauri::command]
pub async fn iphone_backup(app: AppHandle, state: State<'_, AppState>, account: String) -> Result<String, String> {
    let r = state.remote(Some(&account))?;
    let label = format!("Backup of {}", iphone(&r)?.name());
    let job = Job::new(&app, "backup", label, &account, "", "");
    let (id, j) = (job.id.clone(), job.clone());
    job.spawn(app, async move {
        let Remote::IPhone(ph) = &*r else { return };
        if let Err(e) = run(ph, &j).await
            && e != "cancelled"
        {
            j.fail("Backup", e);
        }
    });
    Ok(id)
}

/// The backup an iPhone account reads. Mirrors `IPhoneBackup` in `src/types.ts`.
#[derive(Serialize)]
pub struct BackupInfo {
    /// When it was made (Unix millis).
    pub date: i64,
    /// Bytes on disk (only when asked for: it walks every file).
    pub size: Option<u64>,
    pub encrypted: bool,
    /// The last backup finished (else it was cut off: some files may be missing or older).
    pub complete: bool,
    /// A backup of this device is running.
    pub running: bool,
}

#[tauri::command]
pub async fn iphone_backup_info(
    state: State<'_, AppState>,
    account: String,
    size: bool,
) -> Result<Option<BackupInfo>, String> {
    let r = state.remote(Some(&account))?;
    let ph = iphone(&r)?;
    let Some(dir) = ph.backup_dir().await else { return Ok(None) };
    let running = dir.file_name().is_some_and(|u| busy(&u.to_string_lossy()));
    // A backup that never finished has no Manifest.db: its date is when it started.
    let Ok(t) =
        dir.join("Manifest.db").metadata().or_else(|_| dir.join(INCOMPLETE).metadata()).and_then(|m| m.modified())
    else {
        return Ok(running.then_some(BackupInfo { date: 0, size: None, encrypted: false, complete: false, running }));
    };
    let date = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let encrypted = encrypted(&dir);
    let complete = !dir.join(INCOMPLETE).exists();
    let size = if size {
        let d = dir.clone();
        tokio::task::spawn_blocking(move || {
            walkdir::WalkDir::new(d).into_iter().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
        })
        .await
        .ok()
    } else {
        None
    };
    Ok(Some(BackupInfo { date, size, encrypted, complete, running }))
}

/// Moves the account's backup to the Trash.
#[tauri::command]
pub async fn iphone_delete_backup(state: State<'_, AppState>, account: String) -> Result<(), String> {
    let r = state.remote(Some(&account))?;
    let ph = iphone(&r)?;
    let dir = ph.backup_dir().await.ok_or("There's no backup of this iPhone")?;
    if dir.file_name().is_some_and(|u| busy(&u.to_string_lossy())) {
        return Err("It's being backed up right now".into());
    }
    ph.forget_backup();
    crate::trash::trash_local(vec![dir.to_string_lossy().to_string()]).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archived(size: i64, mtime: i64) -> Vec<u8> {
        let mut obj = plist::Dictionary::new();
        obj.insert("Size".into(), size.into());
        obj.insert("LastModified".into(), mtime.into());
        obj.insert("$class".into(), plist::Value::Uid(plist::Uid::new(2)));
        let mut top = plist::Dictionary::new();
        top.insert("root".into(), plist::Value::Uid(plist::Uid::new(1)));
        let mut d = plist::Dictionary::new();
        d.insert("$archiver".into(), "NSKeyedArchiver".into());
        d.insert("$top".into(), top.into());
        d.insert("$objects".into(), plist::Value::Array(vec!["$null".into(), obj.into()]));
        let mut out = vec![];
        plist::Value::Dictionary(d).to_writer_binary(&mut out).unwrap();
        out
    }

    /// A backup folder with a real SQLite Manifest.db, laid out as the device writes it.
    #[test]
    fn reads_a_backup_by_app() {
        let dir = std::env::temp_dir().join(format!("nx-ibackup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let c = rusqlite::Connection::open(dir.join("Manifest.db")).unwrap();
        c.execute_batch(
            "CREATE TABLE Files (fileID TEXT PRIMARY KEY, domain TEXT, relativePath TEXT, flags INTEGER, file BLOB);",
        )
        .unwrap();
        type TestRow<'a> = (&'a str, &'a str, &'a str, i64, Option<Vec<u8>>);
        let rows: &[TestRow] = &[
            ("d0", "AppDomain-net.whatsapp.WhatsApp", "", 2, None),
            ("d1", "AppDomain-net.whatsapp.WhatsApp", "Documents", 2, Some(archived(0, 1_700_000_000))),
            ("aa11", "AppDomain-net.whatsapp.WhatsApp", "Documents/chat.txt", 1, Some(archived(5, 1_700_000_100))),
            (
                "bb22",
                "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
                "Message/Media/a.jpg",
                1,
                Some(archived(3, 1)),
            ),
            ("cc33", "AppDomainPlugin-net.whatsapp.WhatsApp.Share", "Library/x.plist", 1, Some(archived(2, 1))),
            ("dd44", "AppDomain-org.videolan.vlc-ios", "Documents/film.mp4", 1, Some(archived(9, 1))),
            ("ee55", "HomeDomain", "Library/Preferences/a.plist", 1, Some(archived(1, 1))),
            ("ff66", "AppDomain-com.other.Other", "Documents/link", 4, None),
        ];
        for (id, domain, path, flags, blob) in rows {
            c.execute(
                "INSERT INTO Files VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, domain, path, flags, blob],
            )
            .unwrap();
        }
        drop(c);
        std::fs::create_dir_all(dir.join("aa")).unwrap();
        std::fs::write(dir.join("aa/aa11"), "hello").unwrap();
        let names = HashMap::from([("org.videolan.vlc-ios".to_string(), "VLC".to_string())]);
        std::fs::write(dir.join(APPS_FILE), serde_json::to_vec(&names).unwrap()).unwrap();

        let ix = Index::load(&dir).unwrap();
        assert_eq!(
            ix.containers(),
            ["Backup · Other", "Backup · VLC", "Backup · net.whatsapp.WhatsApp"],
            "links are left out, and an app with nothing left has no container"
        );
        let wa = "Backup · net.whatsapp.WhatsApp";
        let mut top: Vec<String> = ix.list(wa, "").unwrap().into_iter().map(|e| e.key).collect();
        top.sort();
        assert_eq!(
            top,
            [
                "App Group · group.net.whatsapp.WhatsApp.shared/",
                "Documents/",
                "Extension · net.whatsapp.WhatsApp.Share/"
            ]
        );
        let docs = ix.list(wa, "Documents/").unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!((docs[0].name.as_str(), docs[0].size, docs[0].modified), ("chat.txt", 5, Some(1_700_000_100_000)));
        let media = ix.list(wa, "App Group · group.net.whatsapp.WhatsApp.shared/Message/Media/").unwrap();
        assert_eq!(media[0].key, "App Group · group.net.whatsapp.WhatsApp.shared/Message/Media/a.jpg");
        assert_eq!(std::fs::read(ix.file(wa, "Documents/chat.txt").unwrap()).unwrap(), b"hello");
        assert_eq!(ix.list("Backup · Other", "").unwrap()[0].key, "HomeDomain/");

        // A backup that was cut off: still read when its list is readable, explained when not.
        std::fs::write(dir.join(INCOMPLETE), b"").unwrap();
        assert!(Index::load(&dir).is_ok());
        std::fs::write(dir.join("Manifest.db"), b"half written").unwrap();
        assert_eq!(Index::load(&dir).err().as_deref(), Some(UNFINISHED));
        std::fs::remove_file(dir.join(INCOMPLETE)).unwrap();
        assert!(Index::load(&dir).err().unwrap().contains("Couldn't read"));

        // An encrypted backup says so instead of showing nothing.
        let mut m = plist::Dictionary::new();
        m.insert("IsEncrypted".into(), true.into());
        plist::Value::Dictionary(m).to_file_xml(dir.join("Manifest.plist")).unwrap();
        assert_eq!(Index::load(&dir).err().as_deref(), Some(ENCRYPTED));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Against a real device: `NOEXPLORER_TEST_IPHONE_BACKUP=<folder> cargo test iphone_backup_roundtrip --
    /// --nocapture` with an unlocked iPhone that trusts this computer. A whole-device backup goes
    /// into `<folder>/<UDID>` (it can be many GB: pick a folder on a disk with room) and stays
    /// there, so a second run shows how long an incremental one takes.
    #[tokio::test]
    async fn iphone_backup_roundtrip() {
        let Ok(dir) = std::env::var("NOEXPLORER_TEST_IPHONE_BACKUP") else {
            eprintln!("skipping iphone_backup_roundtrip: NOEXPLORER_TEST_IPHONE_BACKUP not set");
            return;
        };
        let _ = ROOT.set(PathBuf::from(dir));
        let ph = IPhone::new(&crate::config::Profile { name: "test".into(), ..Default::default() });
        let job = Job::untracked("backup", "t".into(), "", "", "");
        let t0 = std::time::Instant::now();
        run(&ph, &job).await.unwrap();
        eprintln!(
            "backup: {:?}, {} files, {} bytes received",
            t0.elapsed(),
            job.files_done.load(Ordering::Relaxed),
            job.bytes_done.load(Ordering::Relaxed)
        );
        let root = crate::iphone::ROOT;
        let apps: Vec<String> = ph.list(root, "Backup/").await.unwrap().into_iter().map(|e| e.key).collect();
        eprintln!("Backup/: {apps:?}");
        let first = apps.first().expect("the backup lists apps");
        eprintln!("{first}: {:?}", ph.list(root, first).await.unwrap().iter().map(|e| &e.key).collect::<Vec<_>>());
    }

    #[test]
    fn names_clash_with_their_bundle_id() {
        let names = HashMap::from([
            ("a.notes".to_string(), "Notes".to_string()),
            ("b.notes".to_string(), "Notes".to_string()),
            ("c.other".to_string(), "Other".to_string()),
        ]);
        let n = container_names(&["a.notes", "b.notes", "c.other", "d.x"], &names);
        assert_eq!(n["a.notes"], "Backup · Notes (a.notes)");
        assert_eq!(n["c.other"], "Backup · Other (c.other)");
        assert_eq!(n["d.x"], "Backup · d.x");
    }
}
