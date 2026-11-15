//! iPhone / iPad over a cable (experimental, ADR-054): Apple File Conduit (AFC) through the
//! system's usbmuxd, with the `idevice` crate (pure Rust).
//!
//! - usbmuxd is the OS's: built into macOS, Apple Devices / iTunes on Windows (127.0.0.1:27015),
//!   the `usbmuxd` package on Linux. `USBMUXD_SOCKET_ADDRESS` overrides it, as for libimobiledevice.
//! - Pairing ("Trust This Computer?") is the system's too: we only read the pair record usbmuxd
//!   keeps. Without one, the error tells the user to unlock the phone and tap Trust.
//! - Containers: "Media" (the AFC media area: DCIM, Downloads, Books…) and one per app that shares
//!   files (UIFileSharingEnabled, Finder's "Files" tab), through house_arrest's VendDocuments, whose
//!   files are under `/Documents`. Apps come from installation_proxy's Browse, which we speak
//!   ourselves (a few plist frames) instead of building the crate's feature and its zip support.
//! - AFC is one request at a time per connection, so a few connections per container are kept
//!   open and reused (`idle`); a listing stats its entries over several of them.
//!
//! - A device can vanish at any moment (cable pulled, replugged, usbmuxd restarted), and a socket
//!   to a vanished device doesn't always fail: reads may simply never return. So every request is
//!   [`guarded`]: it stops when usbmuxd no longer lists the device ([`unplugged`]) or after a
//!   deadline, and either counts as a lost connection.
//! - Containers named `Backup · …` are apps' files from a backup this app made
//!   (`iphone_backup.rs`, ADR-055), read from disk: they work with the phone unplugged.
//!
//! The account form maps to: `endpoint` = the device's UDID ("" = whichever one is connected).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use idevice::afc::AfcClient;
use idevice::afc::errors::AfcError;
use idevice::afc::opcode::AfcFopenMode;
use idevice::house_arrest::HouseArrestClient;
use idevice::lockdown::LockdownClient;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection, UsbmuxdDevice};
use idevice::{Idevice, IdeviceError, IdeviceService};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};

use crate::config::Profile;
use crate::iphone_backup::{self as backup, Index};
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry, drop_part, open_part, part_path, resume_offset};
use crate::util::file_name;

/// The one container the UI sees: the device as folders.
pub const ROOT: &str = "iPhone";
/// The media area (an internal container, and its folder at the top).
pub const MEDIA: &str = "Media";
const MEDIA_DIR: &str = "Media/";
const APPS_DIR: &str = "Apps/";
const BACKUP_DIR: &str = "Backup/";
const NOT_HERE: &str = "Open Media or an app's folder first: this level only lists them.";
/// Bytes per AFC read/write (AFC's own limit per packet).
const CHUNK: usize = 1 << 20;
/// Open connections kept per container (and how many a listing uses at once).
const POOL: usize = 4;
/// Shown in the device's logs as the client.
pub(crate) const LABEL: &str = "NoExplorer";

const NONE_CONNECTED: &str = "No iPhone or iPad is connected. Connect it with a cable and unlock it.";
const LOCKED: &str = "Unlock your iPhone, then try again.";
const UNTRUSTED: &str =
    "Your iPhone no longer trusts this computer. Unplug it, plug it back in, unlock it and tap Trust.";
const DISCONNECTED: &str = "The iPhone was disconnected. Connect it again, then try again.";
const STOPPED: &str = "The iPhone stopped answering. Unplug it, plug it back in and unlock it.";
/// One AFC request (or a `with!` body: a folder's worth of stats) may take this long.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Reaching a service: lockdown, its TLS session, starting the service.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Questions to usbmuxd itself (local, normally instant).
const MUX_TIMEOUT: Duration = Duration::from_secs(5);

/// `with!(self, container, |c| body)`: runs `body` (async code using `c: &mut AfcClient` that
/// ends in a `Result<_, IdeviceError>`) on a pooled or new connection to `container`, retrying once
/// on a fresh connection when a pooled one is dead. A macro rather than a method taking an async
/// closure: those futures borrow `c` for any lifetime, and rustc can't prove them `Send`.
macro_rules! with {
    ($me:expr, $container:expr, |$c:ident| $body:expr) => {{
        let me = $me;
        let container: &str = $container;
        let mut attempt = 0;
        loop {
            let pooled = me.pooled(container);
            let fresh = pooled.is_none();
            let mut conn = match pooled {
                Some(c) => c,
                None => match me.connect(container).await {
                    Ok(c) => c,
                    Err(e) => break Err(e),
                },
            };
            let id = me.device_id().await;
            let r = {
                let $c = &mut conn;
                $crate::iphone::guarded(id, REQUEST_TIMEOUT, async { $body }).await
            };
            match me.settle(container, conn, r, !fresh && attempt == 0).await {
                Some(r) => break r,
                None => attempt += 1,
            }
        }
    }};
}

pub struct IPhone {
    profile: Profile,
    /// UDID of the chosen device ("" = whichever is connected).
    udid: String,
    /// The device as usbmuxd last reported it (its id changes when it's plugged in again).
    device: tokio::sync::Mutex<Option<UsbmuxdDevice>>,
    /// Idle AFC connections per container, ready for the next request.
    idle: Mutex<HashMap<String, Vec<AfcClient>>>,
    /// App containers: shown name → bundle id (from the last `containers()`).
    apps: Mutex<HashMap<String, String>>,
    /// The backup's file list, with the `Manifest.db` time it was read at.
    backup: Mutex<Option<(PathBuf, std::time::SystemTime, Arc<Index>)>>,
}

/// A connected device for the account form (`iphone_devices`). Mirrors `IPhoneDevice` in `src/types.ts`.
#[derive(Serialize, Clone, Debug)]
pub struct DeviceInfo {
    pub udid: String,
    /// "Kris's iPhone"; the model when the device won't say.
    pub name: String,
    /// "iPhone", "iPad"…
    pub class: String,
    /// "iPhone15,2"
    pub model: String,
    /// Connected by cable (else over Wi-Fi, through Finder / Apple Devices sync).
    pub usb: bool,
    /// The system has a pair record: the device trusts this computer.
    pub trusted: bool,
}

impl IPhone {
    pub fn new(p: &Profile) -> IPhone {
        IPhone {
            profile: p.clone(),
            udid: p.endpoint.trim().to_string(),
            device: tokio::sync::Mutex::new(None),
            idle: Mutex::new(HashMap::new()),
            apps: Mutex::new(HashMap::new()),
            backup: Mutex::new(None),
        }
    }

    pub fn account(&self) -> &str {
        &self.profile.id
    }

    pub fn name(&self) -> &str {
        &self.profile.name
    }

    /// usbmuxd's id of the device in use (`u32::MAX` when none: [`unplugged`] then ends at once).
    async fn device_id(&self) -> u32 {
        self.device.lock().await.as_ref().map_or(u32::MAX, |d| d.device_id)
    }

    pub(crate) async fn device(&self) -> Result<UsbmuxdDevice, String> {
        let mut g = self.device.lock().await;
        if let Some(d) = g.as_ref() {
            return Ok(d.clone());
        }
        let devs = list_devices().await?;
        let d = pick(devs, &self.udid).ok_or_else(|| {
            if self.udid.is_empty() {
                NONE_CONNECTED.to_string()
            } else {
                format!("{} isn't connected. Connect it with a cable and unlock it.", self.profile.name)
            }
        })?;
        *g = Some(d.clone());
        Ok(d)
    }

    /// Drops every open connection and the device (it was unplugged, or plugged in again).
    async fn forget(&self) {
        self.idle.lock().unwrap().clear();
        *self.device.lock().await = None;
    }

    /// A new AFC connection to `container`.
    async fn connect(&self, container: &str) -> Result<AfcClient, String> {
        let d = self.device().await?;
        let p = d.to_provider(addr(), LABEL);
        let bundle = if container == MEDIA { None } else { Some(self.bundle(container).await?) };
        let res = guarded(d.device_id, CONNECT_TIMEOUT, async {
            match bundle {
                None => AfcClient::connect(&p).await,
                Some(b) => HouseArrestClient::connect(&p).await?.vend_documents(b).await,
            }
        })
        .await;
        match res {
            Ok(c) => Ok(c),
            Err(e) => {
                *self.device.lock().await = None;
                Err(explain(&d, e).await)
            }
        }
    }

    fn pooled(&self, container: &str) -> Option<AfcClient> {
        self.idle.lock().unwrap().get_mut(container).and_then(Vec::pop)
    }

    /// Takes an idle connection, or opens one.
    async fn take(&self, container: &str) -> Result<AfcClient, String> {
        let pooled = self.pooled(container);
        match pooled {
            Some(c) => Ok(c),
            None => self.connect(container).await,
        }
    }

    /// Returns a healthy connection for reuse.
    fn give(&self, container: &str, c: AfcClient) {
        let mut g = self.idle.lock().unwrap();
        let v = g.entry(container.to_string()).or_default();
        if v.len() < POOL {
            v.push(c);
        }
    }

    /// After a request on `c` (see [`with!`]): keeps a healthy connection, and asks for one retry on
    /// a fresh connection when a pooled one turned out dead (unplugged, asleep or replugged since).
    async fn settle<T>(
        &self,
        container: &str,
        c: AfcClient,
        r: Result<T, IdeviceError>,
        may_retry: bool,
    ) -> Option<Result<T, String>> {
        match r {
            Ok(v) => {
                self.give(container, c);
                Some(Ok(v))
            }
            Err(e) if lost(&e) => {
                drop(c);
                self.forget().await;
                if may_retry { None } else { Some(Err(device_err(e))) }
            }
            Err(e) => {
                // An AFC error (not found, exists…) leaves the connection fine.
                if matches!(e, IdeviceError::Afc(_)) {
                    self.give(container, c);
                }
                Some(Err(afc_err(e)))
            }
        }
    }

    /// A request on a taken connection failed: a lost device drops every connection (they're all
    /// on the same cable), and the error is put in the user's terms.
    async fn failed(&self, e: IdeviceError) -> String {
        if lost(&e) {
            self.forget().await;
        }
        afc_err(e)
    }

    /// The device path of `key` in `container`.
    fn path(&self, container: &str, key: &str) -> String {
        afc_path(container == MEDIA, key)
    }

    /// Bundle id behind an app container's name.
    async fn bundle(&self, container: &str) -> Result<String, String> {
        let known = self.apps.lock().unwrap().get(container).cloned();
        if let Some(b) = known {
            return Ok(b);
        }
        self.load_apps().await?;
        let found = self.apps.lock().unwrap().get(container).cloned();
        found.ok_or_else(|| format!("“{container}” isn't on this device, or no longer shares files"))
    }

    /// Lists the apps that share files and remembers their containers' names.
    async fn load_apps(&self) -> Result<Vec<String>, String> {
        let d = self.device().await?;
        let p = d.to_provider(addr(), LABEL);
        let ip = match guarded(d.device_id, CONNECT_TIMEOUT, InstallProxy::connect(&p)).await {
            Ok(ip) => ip,
            Err(e) => return Err(explain(&d, e).await),
        };
        let mut s = ip.0.get_socket().ok_or_else(|| DISCONNECTED.to_string())?;
        let apps = within(d.device_id, browse(&mut s))
            .await?
            .into_iter()
            .filter(|a| a.shares)
            .map(|a| (a.name, a.id))
            .collect();
        let named = container_names(apps);
        let names = named.iter().map(|(n, _)| n.clone()).collect();
        *self.apps.lock().unwrap() = named.into_iter().collect();
        Ok(names)
    }

    /// Every user app as bundle id → name (for reading a backup by app).
    pub(crate) async fn app_names(&self) -> Result<HashMap<String, String>, String> {
        let d = self.device().await?;
        let p = d.to_provider(addr(), LABEL);
        let ip = guarded(d.device_id, CONNECT_TIMEOUT, InstallProxy::connect(&p)).await.map_err(device_err)?;
        let mut s = ip.0.get_socket().ok_or_else(|| DISCONNECTED.to_string())?;
        Ok(within(d.device_id, browse(&mut s)).await?.into_iter().map(|a| (a.id, a.name)).collect())
    }

    /// Reachable now? (Listing the media area also proves the device trusts this computer.)
    async fn alive(&self) -> Result<(), String> {
        with!(self, MEDIA, |c| c.list_dir("/").await.map(|_| ()))
    }

    /// One container: the device as a folder tree (`Media/`, `Apps/`, `Backup/`, see [`route`]).
    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        if let Err(e) = self.alive().await {
            // Unplugged: the backup can still be read.
            if !matches!(self.backup_index().await, Ok(Some(_))) {
                return Err(e);
            }
        }
        Ok(vec![BucketInfo { name: ROOT.into(), region: None }])
    }

    pub async fn list(&self, _container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let dirs =
            |names: Vec<String>, top: &str| names.into_iter().map(|n| folder(&format!("{top}{n}/"), &n)).collect();
        match route(prefix) {
            At::Root => {
                let live = self.alive().await;
                let ix = self.backup_index().await;
                let mut out = vec![];
                if live.is_ok() {
                    out.push(folder(MEDIA_DIR, MEDIA));
                    out.push(folder(APPS_DIR, "Apps"));
                }
                // An unreadable backup (encrypted, unfinished) still shows: opening it says why.
                if !matches!(ix, Ok(None)) {
                    out.push(folder(BACKUP_DIR, "Backup"));
                }
                match live {
                    Err(e) if out.is_empty() => Err(e),
                    _ => Ok(out),
                }
            }
            At::Apps => Ok(dirs(self.load_apps().await?, APPS_DIR)),
            At::Backup => {
                let names = self.saved().await?.containers();
                Ok(dirs(names.iter().map(|n| n.trim_start_matches(backup::PREFIX).to_string()).collect(), BACKUP_DIR))
            }
            At::In { container, base, rest } => {
                let mut out = self.list_in(&container, &rest).await?;
                for e in &mut out {
                    e.key = format!("{base}{}", e.key);
                }
                Ok(out)
            }
            At::Unknown => Err("Not found".into()),
        }
    }

    pub async fn read_range(
        &self,
        _container: &str,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        let (c, rest) = file_at(key)?;
        self.read_range_in(&c, &rest, start, end, cap).await
    }

    pub async fn read_head(&self, container: &str, key: &str, max: u64) -> Result<Vec<u8>, String> {
        Ok(self.read_range(container, key, 0, Some(max.max(1) - 1), max.max(1)).await?.0)
    }

    pub async fn download_file(&self, _container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        let (c, rest) = file_at(key)?;
        self.download_file_in(&c, &rest, path, job).await
    }

    pub async fn upload_file(
        &self,
        _container: &str,
        key: &str,
        path: &Path,
        size: u64,
        job: &Job,
    ) -> Result<(), String> {
        let (c, rest) = file_at(key).map_err(|_| NOT_HERE.to_string())?;
        self.upload_file_in(&c, &rest, path, size, job).await
    }

    pub async fn mkdir(&self, _container: &str, prefix: &str, name: &str) -> Result<(), String> {
        match route(prefix) {
            At::In { container, rest, .. } => self.mkdir_in(&container, &rest, name).await,
            _ => Err(NOT_HERE.into()),
        }
    }

    pub async fn delete(&self, _container: &str, keys: Vec<String>, job: Option<&Job>) -> Result<usize, String> {
        let mut container = None;
        let mut rests = vec![];
        for k in &keys {
            let (c, rest) = item_at(k)?;
            if container.get_or_insert_with(|| c.clone()) != &c {
                return Err("Delete from one place at a time".into());
            }
            rests.push(rest);
        }
        let Some(c) = container else { return Ok(0) };
        self.delete_in(&c, rests, job).await
    }

    pub async fn transfer_within(
        &self,
        _container: &str,
        from: &str,
        to: &str,
        keep: bool,
        job: Option<&Job>,
    ) -> Result<(), String> {
        let (sc, a) = item_at(from)?;
        let (dc, b) = item_at(to).map_err(|_| NOT_HERE.to_string())?;
        self.transfer_within_in((&sc, &a), (&dc, &b), keep, job).await
    }

    // ---------- the backup (iphone_backup.rs) ----------

    /// Which device's backup this account reads: its own, the connected one, else the latest.
    pub(crate) async fn backup_dir(&self) -> Option<PathBuf> {
        let udid = if !self.udid.is_empty() {
            self.udid.clone()
        } else if let Ok(d) = self.device().await {
            d.udid
        } else {
            backup::latest()?
        };
        Some(backup::root().ok()?.join(udid))
    }

    /// The backup's file list (None = no backup yet), read again when the backup changed. During a
    /// backup the list read before it is kept: the folder is being rewritten.
    async fn backup_index(&self) -> Result<Option<Arc<Index>>, String> {
        let Some(dir) = self.backup_dir().await else { return Ok(None) };
        let running = dir.file_name().is_some_and(|u| backup::busy(&u.to_string_lossy()));
        let Ok(t) = dir.join("Manifest.db").metadata().and_then(|m| m.modified()) else {
            // A first backup that never finished leaves files but no list: say so (Backup/ shows,
            // and the right-click menu can delete it).
            return match backup::unfinished(&dir) {
                false => Ok(None),
                true if running => Err(backup::RUNNING.into()),
                true => Err(backup::UNFINISHED.into()),
            };
        };
        if let Some((d, at, ix)) = self.backup.lock().unwrap().as_ref()
            && *d == dir
            && (*at == t || running)
        {
            return Ok(Some(ix.clone()));
        }
        let d = dir.clone();
        let ix = Arc::new(tokio::task::spawn_blocking(move || Index::load(&d)).await.map_err(|e| e.to_string())??);
        *self.backup.lock().unwrap() = Some((dir, t, ix.clone()));
        Ok(Some(ix))
    }

    /// After a backup or before deleting it: read it again next time.
    pub(crate) fn forget_backup(&self) {
        *self.backup.lock().unwrap() = None;
    }

    async fn saved(&self) -> Result<Arc<Index>, String> {
        self.backup_index().await?.ok_or_else(|| "There's no backup of this iPhone any more".into())
    }

    async fn saved_file(&self, container: &str, key: &str) -> Result<PathBuf, String> {
        self.saved().await?.file(container, key)
    }

    async fn list_in(&self, container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        if backup::is_backup(container) {
            return self.saved().await?.list(container, prefix);
        }
        let dir = self.path(container, prefix);
        let names: Vec<String> = with!(self, container, |c| c.list_dir(dir.clone()).await)?
            .into_iter()
            // Uploads cut off by an unplugged cable leave their temp file behind: not the user's.
            .filter(|n| n != "." && n != ".." && !is_temp(n))
            .collect();
        // One stat per entry: spread over a few connections (thousands of photos in DCIM).
        let per = names.len().div_ceil(POOL).max(1);
        let parts = futures::future::join_all(names.chunks(per).map(|chunk| {
            let dir = dir.clone();
            async move {
                with!(self, container, |c| async {
                    let mut out = Vec::with_capacity(chunk.len());
                    for n in chunk {
                        out.push((n.clone(), stat(c, &dir, n).await?));
                    }
                    Ok::<_, IdeviceError>(out)
                }
                .await)
            }
        }))
        .await;
        let mut out = vec![];
        for part in parts {
            for (name, st) in part? {
                // Gone between the listing and the stat, or unreadable: leave it out.
                let Some((dir_, size, modified)) = st else { continue };
                out.push(Entry {
                    key: format!("{prefix}{name}{}", if dir_ { "/" } else { "" }),
                    name,
                    size,
                    modified,
                    dir: dir_,
                    hash: None,
                });
            }
        }
        Ok(out)
    }

    async fn read_range_in(
        &self,
        container: &str,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        if backup::is_backup(container) {
            return read_saved(&self.saved_file(container, key).await?, start, end, cap).await;
        }
        let p = self.path(container, key);
        with!(self, container, |c| async {
            let total = c.get_file_info(p.clone()).await?.size as u64;
            let last = end.unwrap_or(start + cap - 1).min(start + cap - 1).min(total.saturating_sub(1));
            let want = (last + 1).saturating_sub(start) as usize;
            let mut f = c.open(p.clone(), AfcFopenMode::RdOnly).await?;
            if start > 0 {
                f.seek(std::io::SeekFrom::Start(start)).await?;
            }
            let bytes = if want > 0 { f.read_n(want).await? } else { vec![] };
            f.close().await?;
            Ok::<_, IdeviceError>((bytes, Some(total)))
        }
        .await)
    }

    async fn mkdir_in(&self, container: &str, prefix: &str, name: &str) -> Result<(), String> {
        read_only(container)?;
        let p = self.path(container, &format!("{prefix}{name}"));
        with!(self, container, |c| c.mk_dir(p.clone()).await)
    }

    async fn exists(&self, container: &str, key: &str) -> Result<bool, String> {
        let p = self.path(container, key);
        with!(self, container, |c| match c.get_file_info(p.clone()).await {
            Ok(_) => Ok(true),
            Err(IdeviceError::Afc(AfcError::ObjectNotFound)) => Ok(false),
            Err(e) => Err(e),
        })
    }

    /// Creates every missing folder above `key`.
    async fn ensure_parents(&self, container: &str, key: &str) -> Result<(), String> {
        let parts: Vec<&str> = key.trim_end_matches('/').split('/').collect();
        let dirs: Vec<String> = (1..parts.len()).map(|i| self.path(container, &parts[..i].join("/"))).collect();
        if dirs.is_empty() {
            return Ok(());
        }
        with!(self, container, |c| async {
            for d in &dirs {
                match c.mk_dir(d.clone()).await {
                    Ok(()) | Err(IdeviceError::Afc(AfcError::ObjectExists)) => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        }
        .await)
    }

    async fn delete_in(&self, container: &str, keys: Vec<String>, job: Option<&Job>) -> Result<usize, String> {
        read_only(container)?;
        let total = keys.len() as u64;
        if let Some(j) = job {
            j.files_total.store(total, Ordering::Relaxed);
        }
        for (i, k) in keys.iter().enumerate() {
            if let Some(j) = job {
                if j.cancelled() {
                    return Err("cancelled".into());
                }
                j.files_done.store(i as u64, Ordering::Relaxed);
            }
            let p = self.path(container, k);
            with!(self, container, |c| match c.remove_all(p.clone()).await {
                Ok(()) | Err(IdeviceError::Afc(AfcError::ObjectNotFound)) => Ok(()),
                Err(e) => Err(e),
            })?;
        }
        if let Some(j) = job {
            j.files_done.store(total, Ordering::Relaxed);
        }
        Ok(keys.len())
    }

    /// Copies one file on the device, possibly between the media area and an app (AFC has no copy:
    /// the bytes pass through the app).
    async fn copy_file(&self, sc: &str, from: &str, dc: &str, to: &str) -> Result<(), String> {
        let (a, b) = (self.path(sc, from), self.path(dc, to));
        let mut src = self.take(sc).await?;
        let mut dst = self.take(dc).await?;
        let id = self.device_id().await;
        let res: Result<(), IdeviceError> = async {
            let mut r = dev!(id, src.open(a, AfcFopenMode::RdOnly))?;
            let mut w = dev!(id, dst.open(b, AfcFopenMode::WrOnly))?;
            loop {
                let chunk = dev!(id, r.read_n(CHUNK))?;
                if chunk.is_empty() {
                    break;
                }
                dev!(id, w.write_entire(&chunk))?;
            }
            dev!(id, w.close())?;
            dev!(id, r.close())
        }
        .await;
        if let Err(e) = res {
            return Err(self.failed(e).await);
        }
        self.give(sc, src);
        self.give(dc, dst);
        Ok(())
    }

    /// Copies a file or folder from container `sc` to `dc`. From the backup, files are uploaded
    /// from their backed-up copies.
    async fn copy_tree(&self, sc: &str, from: &str, dc: &str, to: &str, job: &Job) -> Result<(), String> {
        if !from.ends_with('/') {
            if backup::is_backup(sc) {
                let f = self.saved_file(sc, from).await?;
                let size = tokio::fs::metadata(&f).await.map(|m| m.len()).unwrap_or(0);
                return self.upload_file_in(dc, to, &f, size, job).await;
            }
            return self.copy_file(sc, from, dc, to).await;
        }
        let p = self.path(dc, to);
        with!(self, dc, |c| match c.mk_dir(p.clone()).await {
            Ok(()) | Err(IdeviceError::Afc(AfcError::ObjectExists)) => Ok(()),
            Err(e) => Err(e),
        })?;
        for e in self.list_in(sc, from).await? {
            if job.cancelled() {
                return Err("cancelled".into());
            }
            let target = format!("{to}{}{}", e.name, if e.dir { "/" } else { "" });
            Box::pin(self.copy_tree(sc, &e.key, dc, &target, job)).await?;
        }
        Ok(())
    }

    async fn transfer_within_in(
        &self,
        (sc, from): (&str, &str),
        (dc, to): (&str, &str),
        keep: bool,
        job: Option<&Job>,
    ) -> Result<(), String> {
        read_only(dc)?;
        if !keep && sc != dc {
            return Err("Moving between Media, an app and the backup isn't possible: copy, then delete".into());
        }
        if !keep {
            read_only(sc)?;
        }
        if self.exists(dc, to).await? {
            return Err(format!("“{}” already exists there", file_name(to)));
        }
        if let Some(j) = job {
            j.files_total.store(1, Ordering::Relaxed);
        }
        self.ensure_parents(dc, to).await?;
        if keep {
            let own;
            let j = match job {
                Some(j) => j,
                None => {
                    own = Job::untracked("dup", String::new(), self.account(), ROOT, "");
                    &own
                }
            };
            self.copy_tree(sc, from, dc, to, j).await?
        } else {
            let (a, b) = (self.path(sc, from), self.path(dc, to));
            with!(self, sc, |c| c.rename(a.clone(), b.clone()).await)?
        }
        if let Some(j) = job {
            j.files_done.store(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Writes to a temporary name next to the target and renames it into place, so an interrupted
    /// upload never leaves a cut-off file under the real name.
    async fn upload_file_in(
        &self,
        container: &str,
        key: &str,
        path: &Path,
        _size: u64,
        job: &Job,
    ) -> Result<(), String> {
        read_only(container)?;
        self.ensure_parents(container, key).await?;
        let target = self.path(container, key);
        let tmp = temp_name(&target);
        let mut local = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
        let mut c = self.take(container).await?;
        let id = self.device_id().await;
        let mut buf = vec![0u8; CHUNK];
        let res: Result<(), Fail> = async {
            let mut f = dev!(id, c.open(tmp.clone(), AfcFopenMode::WrOnly))?;
            loop {
                if job.cancelled() {
                    return Err(Fail::Other("cancelled".into()));
                }
                let n = local.read(&mut buf).await.map_err(|e| Fail::Other(e.to_string()))?;
                if n == 0 {
                    break;
                }
                dev!(id, f.write_entire(&buf[..n]))?;
                job.flow(n as u64).await;
            }
            dev!(id, f.close())?;
            // Uploads replace an existing file, like on every other provider.
            match dev!(id, c.rename(tmp.clone(), target.clone())) {
                Err(IdeviceError::Afc(AfcError::ObjectExists)) => {
                    dev!(id, c.remove(target.clone()))?;
                    Ok(dev!(id, c.rename(tmp.clone(), target.clone()))?)
                }
                r => Ok(r?),
            }
        }
        .await;
        let res = match res {
            Ok(()) => Ok(()),
            Err(Fail::Device(e)) => Err(self.failed(e).await),
            Err(Fail::Other(e)) => Err(e),
        };
        match res {
            Ok(()) => {
                self.give(container, c);
                Ok(())
            }
            Err(e) => {
                drop(c); // it may still hold the open file
                // Gone with the device otherwise: the next listing hides it, the next upload replaces it.
                if e != DISCONNECTED && e != STOPPED {
                    let _ = with!(self, container, |c| c.remove(tmp.clone()).await);
                }
                Err(e)
            }
        }
    }

    async fn download_file_in(&self, container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let tmp = part_path(path);
        let offset = resume_offset(&tmp, job).await;
        if backup::is_backup(container) {
            let from = self.saved_file(container, key).await?;
            return copy_saved(&from, &tmp, path, offset, job).await;
        }
        let src = self.path(container, key);
        let mut c = self.take(container).await?;
        let id = self.device_id().await;
        let other = |e: std::io::Error| Fail::Other(e.to_string());
        let res: Result<(), Fail> = async {
            let mut f = dev!(id, c.open(src, AfcFopenMode::RdOnly))?;
            if offset > 0 {
                guarded(id, REQUEST_TIMEOUT, async { Ok(f.seek(std::io::SeekFrom::Start(offset)).await?) }).await?;
            }
            let file = open_part(&tmp, offset > 0, offset, job).await.map_err(Fail::Other)?;
            let mut w = tokio::io::BufWriter::with_capacity(1 << 20, file);
            loop {
                if job.cancelled() {
                    return Err(Fail::Other("cancelled".into()));
                }
                let chunk = dev!(id, f.read_n(CHUNK))?;
                if chunk.is_empty() {
                    break;
                }
                w.write_all(&chunk).await.map_err(other)?;
                job.flow(chunk.len() as u64).await;
            }
            w.flush().await.map_err(other)?;
            Ok(dev!(id, f.close())?)
        }
        .await;
        let res = match res {
            Ok(()) => Ok(()),
            Err(Fail::Device(e)) => Err(self.failed(e).await),
            Err(Fail::Other(e)) => Err(e),
        };
        match res {
            Ok(()) => {
                self.give(container, c);
                tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string())
            }
            Err(e) => {
                drop(c);
                drop_part(&tmp, job, &e).await;
                Err(e)
            }
        }
    }
}

/// Where a key of the folder tree goes: the top level, the `Apps/` and `Backup/` lists, or a key
/// `rest` in an internal container (the media area, an app's Documents, an app in the backup),
/// whose keys get `base` in front.
#[derive(Debug, PartialEq)]
enum At {
    Root,
    Apps,
    Backup,
    In { container: String, base: String, rest: String },
    Unknown,
}

fn route(key: &str) -> At {
    let k = key.trim_start_matches('/');
    if k.is_empty() {
        return At::Root;
    }
    let (top, rest) = k.split_once('/').unwrap_or((k, ""));
    if top == MEDIA {
        return At::In { container: MEDIA.into(), base: MEDIA_DIR.into(), rest: rest.into() };
    }
    let backup = match top {
        "Apps" => false,
        "Backup" => true,
        _ => return At::Unknown,
    };
    if rest.is_empty() {
        return if backup { At::Backup } else { At::Apps };
    }
    let (name, rest) = rest.split_once('/').unwrap_or((rest, ""));
    At::In {
        container: if backup { format!("{}{name}", backup::PREFIX) } else { name.to_string() },
        base: format!("{top}/{name}/"),
        rest: rest.into(),
    }
}

/// A file, or a folder inside a container (not one of the top-level folders, which can't be
/// renamed, deleted or written to).
fn item_at(key: &str) -> Result<(String, String), String> {
    match route(key) {
        At::In { container, rest, .. } if !rest.trim_matches('/').is_empty() => Ok((container, rest)),
        At::Unknown => Err("Not found".into()),
        _ => Err("Media, Apps, Backup and the apps' own folders can't be changed".into()),
    }
}

fn file_at(key: &str) -> Result<(String, String), String> {
    if key.ends_with('/') {
        return Err("That's a folder".into());
    }
    item_at(key)
}

fn folder(key: &str, name: &str) -> Entry {
    Entry { key: key.into(), name: name.into(), size: 0, modified: None, dir: true, hash: None }
}

/// A step of a transfer: the device's error (to tell a lost cable apart), or ours.
enum Fail {
    Device(IdeviceError),
    Other(String),
}

impl From<IdeviceError> for Fail {
    fn from(e: IdeviceError) -> Fail {
        Fail::Device(e)
    }
}

/// `dev!(id, future)`: one guarded device request (see [`guarded`]).
macro_rules! dev {
    ($id:expr, $f:expr) => {
        $crate::iphone::guarded($id, REQUEST_TIMEOUT, $f).await
    };
}
use dev;

/// Resolves once usbmuxd no longer lists device `id`: unplugged, or plugged in again (it then gets
/// a new id, and connections to the old one are dead). Also when usbmuxd itself is gone.
pub(crate) async fn unplugged(id: u32) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let Ok(Ok(mut m)) = tokio::time::timeout(MUX_TIMEOUT, addr().connect(0)).await else { return };
        // usbmuxd slow to answer isn't the device leaving: ask again.
        if let Ok(Ok(devs)) = tokio::time::timeout(MUX_TIMEOUT, m.get_devices()).await
            && !devs.iter().any(|d| d.device_id == id)
        {
            return;
        }
    }
}

/// Runs a device request until it ends, the device is unplugged, or `limit` passes. The last two
/// are lost connections: [`device_err`] tells the user which.
pub(crate) async fn guarded<T>(
    id: u32,
    limit: Duration,
    f: impl Future<Output = Result<T, IdeviceError>>,
) -> Result<T, IdeviceError> {
    tokio::select! {
        r = f => r,
        _ = unplugged(id) => Err(std::io::Error::from(std::io::ErrorKind::NotConnected).into()),
        _ = tokio::time::sleep(limit) => Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into()),
    }
}

/// [`guarded`] for our own exchanges, which fail with a message.
async fn within<T>(id: u32, f: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    guarded(id, REQUEST_TIMEOUT, async { f.await.map_err(IdeviceError::InternalError) }).await.map_err(|e| match e {
        IdeviceError::InternalError(m) => m,
        e => device_err(e),
    })
}

/// usbmuxd's device list, with a deadline.
async fn list_devices() -> Result<Vec<UsbmuxdDevice>, String> {
    let mut m = mux().await?;
    match tokio::time::timeout(MUX_TIMEOUT, m.get_devices()).await {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err(no_service()),
    }
}

/// An upload's temporary name (see [`temp_name`]).
fn is_temp(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".noexplorer-part")
}

fn read_only(container: &str) -> Result<(), String> {
    if backup::is_backup(container) { Err(backup::READ_ONLY.into()) } else { Ok(()) }
}

/// Bytes `start..` (at most `cap`, up to `end`) of a file in a backup, and its size.
async fn read_saved(f: &Path, start: u64, end: Option<u64>, cap: u64) -> Result<(Vec<u8>, Option<u64>), String> {
    let mut file = tokio::fs::File::open(f).await.map_err(|_| "Not found".to_string())?;
    let total = file.metadata().await.map_err(|e| e.to_string())?.len();
    let last = end.unwrap_or(start + cap - 1).min(start + cap - 1).min(total.saturating_sub(1));
    let want = (last + 1).saturating_sub(start);
    file.seek(std::io::SeekFrom::Start(start)).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(want as usize);
    file.take(want).read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    Ok((buf, Some(total)))
}

/// A download from a backup: a local copy, through the `.part` like any download.
async fn copy_saved(from: &Path, tmp: &Path, path: &Path, offset: u64, job: &Job) -> Result<(), String> {
    let res: Result<(), String> = async {
        let mut src = tokio::fs::File::open(from).await.map_err(|_| "Not found".to_string())?;
        if offset > 0 {
            src.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| e.to_string())?;
        }
        let mut w = open_part(tmp, offset > 0, offset, job).await?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if job.cancelled() {
                return Err("cancelled".into());
            }
            let n = src.read(&mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
            job.flow(n as u64).await;
        }
        w.flush().await.map_err(|e| e.to_string())
    }
    .await;
    match res {
        Ok(()) => tokio::fs::rename(tmp, path).await.map_err(|e| e.to_string()),
        Err(e) => {
            drop_part(tmp, job, &e).await;
            Err(e)
        }
    }
}

/// `(is a folder, size, modified millis)` of `dir/name`, following a link to see whether it's a
/// folder. None: it's gone, or the device won't say.
async fn stat(c: &mut AfcClient, dir: &str, name: &str) -> Result<Option<(bool, u64, Option<i64>)>, IdeviceError> {
    let p = join(dir, name);
    let info = match c.get_file_info(p).await {
        Ok(i) => i,
        Err(IdeviceError::Afc(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut is_dir = info.st_ifmt == "S_IFDIR";
    if info.st_ifmt == "S_IFLNK"
        && let Some(t) = &info.st_link_target
    {
        let target = if t.starts_with('/') { t.clone() } else { join(dir, t) };
        match c.get_file_info(target).await {
            Ok(i) => is_dir = i.st_ifmt == "S_IFDIR",
            Err(IdeviceError::Afc(_)) => {}
            Err(e) => return Err(e),
        }
    }
    let modified = Some(info.modified.and_utc().timestamp_millis());
    Ok(Some((is_dir, if is_dir { 0 } else { info.size as u64 }, modified)))
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

/// Device path of a key: the media area's root is `/`, an app's is its `/Documents`. `.` and `..`
/// are dropped, so a key can't leave its container.
fn afc_path(media: bool, key: &str) -> String {
    let rest: Vec<&str> = key.split('/').filter(|s| !s.is_empty() && *s != "." && *s != "..").collect();
    let base = if media { "" } else { "/Documents" };
    match (rest.is_empty(), media) {
        (true, true) => "/".into(),
        (true, false) => base.into(),
        _ => format!("{base}/{}", rest.join("/")),
    }
}

/// `/a/b/photo.jpg` → `/a/b/.photo.jpg.noexplorer-part`
fn temp_name(path: &str) -> String {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    format!("{dir}/.{name}.noexplorer-part")
}

/// The connection to look at: the chosen UDID ("" = any), by cable before Wi-Fi.
fn pick(devs: Vec<UsbmuxdDevice>, udid: &str) -> Option<UsbmuxdDevice> {
    let mut m: Vec<UsbmuxdDevice> = devs.into_iter().filter(|d| udid.is_empty() || d.udid == udid).collect();
    m.sort_by_key(|d| d.connection_type != Connection::Usb);
    m.into_iter().next()
}

pub(crate) fn addr() -> UsbmuxdAddr {
    UsbmuxdAddr::from_env_var().unwrap_or_default()
}

async fn mux() -> Result<UsbmuxdConnection, String> {
    match tokio::time::timeout(MUX_TIMEOUT, addr().connect(0)).await {
        Ok(Ok(m)) => Ok(m),
        _ => Err(no_service()),
    }
}

fn no_service() -> String {
    if cfg!(windows) {
        "Can't reach Apple's device service. Install Apple Devices (Microsoft Store) or iTunes, then connect your iPhone again.".into()
    } else if cfg!(target_os = "macos") {
        "Can't reach macOS's device service (usbmuxd).".into()
    } else {
        "Can't reach usbmuxd. Install it (e.g. sudo apt install usbmuxd), then connect your iPhone again.".into()
    }
}

fn trust_hint() -> String {
    let more = if cfg!(windows) { " If it doesn't ask, open Apple Devices with the iPhone connected." } else { "" };
    format!("Unlock your iPhone and tap Trust when it asks about this computer, then try again.{more}")
}

/// Why a connection to the device failed, in the user's terms.
pub(crate) async fn explain(d: &UsbmuxdDevice, e: IdeviceError) -> String {
    match e {
        IdeviceError::DeviceLocked => LOCKED.into(),
        IdeviceError::InvalidHostID => UNTRUSTED.into(),
        e => {
            // No pair record: the device was never trusted here.
            let trusted = match mux().await {
                Ok(mut m) => !matches!(tokio::time::timeout(MUX_TIMEOUT, m.get_pair_record(&d.udid)).await, Ok(Err(_))),
                Err(_) => true,
            };
            if trusted { device_err(e) } else { trust_hint() }
        }
    }
}

/// The connection itself is gone (unplugged, asleep), as opposed to a refused request.
fn lost(e: &IdeviceError) -> bool {
    matches!(e, IdeviceError::Socket(_) | IdeviceError::NoEstablishedConnection | IdeviceError::Usbmuxd(_))
}

pub(crate) fn device_err(e: IdeviceError) -> String {
    match &e {
        IdeviceError::Socket(io) if io.kind() == std::io::ErrorKind::TimedOut => STOPPED.into(),
        e if lost(e) => DISCONNECTED.into(),
        e => format!("iPhone: {e}"),
    }
}

fn afc_err(e: IdeviceError) -> String {
    match e {
        IdeviceError::Afc(AfcError::ObjectNotFound) => "Not found".into(),
        IdeviceError::Afc(AfcError::PermDenied) => "The iPhone doesn't allow that here".into(),
        IdeviceError::Afc(AfcError::NoSpaceLeft) => "The iPhone is full".into(),
        IdeviceError::Afc(AfcError::ObjectExists) => "Already exists".into(),
        IdeviceError::Afc(AfcError::DirNotEmpty) => "The folder isn't empty".into(),
        e => device_err(e),
    }
}

// ---------- apps that share files (installation_proxy) ----------

/// installation_proxy, only far enough to list apps: the plist frames are read and written here.
struct InstallProxy(Idevice);

impl IdeviceService for InstallProxy {
    fn service_name() -> std::borrow::Cow<'static, str> {
        "com.apple.mobile.installation_proxy".into()
    }

    async fn from_stream(idevice: Idevice) -> Result<Self, IdeviceError> {
        Ok(InstallProxy(idevice))
    }
}

/// Lockdown services frame each plist with its length (4 bytes, big-endian).
async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, v: &plist::Value) -> Result<(), String> {
    let mut buf = vec![];
    v.to_writer_xml(&mut buf).map_err(|e| e.to_string())?;
    w.write_all(&(buf.len() as u32).to_be_bytes()).await.map_err(|e| e.to_string())?;
    w.write_all(&buf).await.map_err(|e| e.to_string())?;
    w.flush().await.map_err(|e| e.to_string())
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<plist::Dictionary, String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(|_| DISCONNECTED.to_string())?;
    let n = u32::from_be_bytes(len) as usize;
    if n > 64 << 20 {
        return Err("iPhone: oversized answer".into());
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await.map_err(|_| DISCONNECTED.to_string())?;
    plist::from_bytes(&buf).map_err(|e| format!("iPhone: {e}"))
}

/// A user app (installation_proxy Browse).
#[derive(Debug, PartialEq)]
struct App {
    name: String,
    id: String,
    /// UIFileSharingEnabled: its Documents folder is reachable (house_arrest VendDocuments).
    shares: bool,
}

/// Every user app.
async fn browse<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> Result<Vec<App>, String> {
    let attrs = ["CFBundleIdentifier", "CFBundleDisplayName", "CFBundleName", "UIFileSharingEnabled"];
    let mut opts = plist::Dictionary::new();
    opts.insert("ApplicationType".into(), "User".into());
    opts.insert("ReturnAttributes".into(), plist::Value::Array(attrs.iter().map(|a| (*a).into()).collect()));
    let mut req = plist::Dictionary::new();
    req.insert("Command".into(), "Browse".into());
    req.insert("ClientOptions".into(), opts.into());
    write_frame(s, &req.into()).await?;
    let mut out = vec![];
    loop {
        let msg = read_frame(s).await?;
        if let Some(e) = msg.get("Error").and_then(|e| e.as_string()) {
            return Err(format!("iPhone: couldn't list apps ({e})"));
        }
        for app in msg.get("CurrentList").and_then(|l| l.as_array()).into_iter().flatten() {
            let Some(d) = app.as_dictionary() else { continue };
            let shares = match d.get("UIFileSharingEnabled") {
                Some(plist::Value::Boolean(b)) => *b,
                Some(plist::Value::String(s)) => matches!(s.to_ascii_lowercase().as_str(), "yes" | "true" | "1"),
                _ => false,
            };
            let Some(id) = d.get("CFBundleIdentifier").and_then(|v| v.as_string()) else { continue };
            let name = ["CFBundleDisplayName", "CFBundleName"]
                .iter()
                .find_map(|k| d.get(k).and_then(|v| v.as_string()).filter(|n| !n.trim().is_empty()))
                .unwrap_or(id);
            out.push(App { name: name.trim().to_string(), id: id.to_string(), shares });
        }
        if msg.get("Status").and_then(|s| s.as_string()) == Some("Complete") {
            return Ok(out);
        }
    }
}

/// Container names for apps, sorted: the app's name, with its bundle id when two share a name
/// (or one is called "Media").
fn container_names(mut apps: Vec<(String, String)>) -> Vec<(String, String)> {
    apps.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then(a.1.cmp(&b.1)));
    let count = |n: &str| apps.iter().filter(|(m, _)| m.eq_ignore_ascii_case(n)).count();
    apps.iter()
        .map(|(n, id)| {
            let clash = count(n) > 1 || n.eq_ignore_ascii_case(MEDIA) || n.starts_with(backup::PREFIX);
            let n = n.replace('/', "∕");
            (if clash { format!("{n} ({id})") } else { n }, id.clone())
        })
        .collect()
}

// ---------- commands ----------

/// iPhones and iPads usbmuxd sees right now (one row per device, cable first).
#[tauri::command]
pub async fn iphone_devices() -> Result<Vec<DeviceInfo>, String> {
    let devs = list_devices().await?;
    let mut udids: Vec<String> = devs.iter().map(|d| d.udid.clone()).collect();
    udids.dedup();
    let mut out = vec![];
    for udid in udids {
        let Some(d) = pick(devs.clone(), &udid) else { continue };
        if out.iter().any(|o: &DeviceInfo| o.udid == udid) {
            continue;
        }
        let trusted = match mux().await {
            Ok(mut m) => matches!(tokio::time::timeout(MUX_TIMEOUT, m.get_pair_record(&udid)).await, Ok(Ok(_))),
            Err(_) => false,
        };
        // Name and model need no pairing (like `ideviceinfo -s`).
        let p = d.to_provider(addr(), LABEL);
        let mut l = tokio::time::timeout(MUX_TIMEOUT, LockdownClient::connect(&p)).await.ok().and_then(Result::ok);
        let model = value(&mut l, "ProductType").await.unwrap_or_default();
        let class = value(&mut l, "DeviceClass").await.unwrap_or_else(|| "iPhone".into());
        let name = value(&mut l, "DeviceName").await.filter(|n| !n.is_empty()).unwrap_or_else(|| class.clone());
        out.push(DeviceInfo { udid, name, class, model, usb: d.connection_type == Connection::Usb, trusted });
    }
    Ok(out)
}

async fn value(l: &mut Option<LockdownClient>, key: &str) -> Option<String> {
    let v = tokio::time::timeout(MUX_TIMEOUT, l.as_mut()?.get_value(Some(key), None)).await.ok()?.ok()?;
    v.as_string().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_in_their_container() {
        assert_eq!(afc_path(true, ""), "/");
        assert_eq!(afc_path(true, "DCIM/100APPLE/"), "/DCIM/100APPLE");
        assert_eq!(afc_path(false, ""), "/Documents");
        assert_eq!(afc_path(false, "Movies/a b.mp4"), "/Documents/Movies/a b.mp4");
        assert_eq!(afc_path(false, "../Library/x"), "/Documents/Library/x");
        assert_eq!(afc_path(true, "./a//../b"), "/a/b");
        assert_eq!(temp_name("/Documents/a.mp4"), "/Documents/.a.mp4.noexplorer-part");
        assert_eq!(join("/", "DCIM"), "/DCIM");
    }

    /// A request that never answers (a socket to an unplugged phone) ends: at the deadline, or when
    /// usbmuxd no longer lists the device (an id nothing has; without usbmuxd, at once).
    #[tokio::test]
    async fn hung_requests_end() {
        let never = std::future::pending::<Result<(), IdeviceError>>;
        let t0 = std::time::Instant::now();
        let e = guarded(1, Duration::from_millis(50), never()).await.unwrap_err();
        assert!(t0.elapsed() < Duration::from_secs(1));
        assert!(lost(&e));
        assert_eq!(device_err(e), STOPPED);
        let t0 = std::time::Instant::now();
        let e = guarded(u32::MAX, Duration::from_secs(60), never()).await.unwrap_err();
        assert!(t0.elapsed() < Duration::from_secs(15), "unplugged device not noticed");
        assert_eq!(device_err(e), DISCONNECTED);
        assert!(guarded(1, Duration::from_secs(5), async { Ok(7) }).await.unwrap() == 7);
    }

    #[test]
    fn upload_leftovers_are_hidden() {
        assert!(is_temp(&temp_name("/Documents/a.mp4")[11..]));
        assert!(!is_temp("a.noexplorer-part") && !is_temp(".hidden"));
    }

    #[test]
    fn the_device_is_a_folder_tree() {
        let into =
            |c: &str, base: &str, rest: &str| At::In { container: c.into(), base: base.into(), rest: rest.into() };
        assert_eq!(route(""), At::Root);
        assert_eq!(route("Apps/"), At::Apps);
        assert_eq!(route("Backup/"), At::Backup);
        assert_eq!(route("Media/DCIM/a.jpg"), into(MEDIA, "Media/", "DCIM/a.jpg"));
        assert_eq!(route("Apps/VLC/Movies/"), into("VLC", "Apps/VLC/", "Movies/"));
        assert_eq!(route("Backup/WhatsApp/Documents/x"), into("Backup · WhatsApp", "Backup/WhatsApp/", "Documents/x"));
        assert_eq!(route("Elsewhere/x"), At::Unknown);
        assert!(item_at("Apps/VLC/").is_err() && item_at("Media/").is_err() && item_at("Apps/").is_err());
        assert_eq!(item_at("Apps/VLC/a.mp4").unwrap(), ("VLC".into(), "a.mp4".into()));
        assert!(file_at("Media/DCIM/").is_err());
    }

    #[test]
    fn picks_the_chosen_device_by_cable_first() {
        let d = |udid: &str, usb: bool| UsbmuxdDevice {
            connection_type: if usb { Connection::Usb } else { Connection::Network("10.0.0.2".parse().unwrap()) },
            udid: udid.into(),
            device_id: 1,
        };
        let devs = vec![d("A", false), d("B", false), d("A", true)];
        let a = pick(devs.clone(), "A").unwrap();
        assert_eq!((a.udid.as_str(), a.connection_type == Connection::Usb), ("A", true));
        assert_eq!(pick(devs.clone(), "").unwrap().udid, "A");
        assert_eq!(pick(devs.clone(), "B").unwrap().udid, "B");
        assert!(pick(devs, "C").is_none());
    }

    #[test]
    fn app_names_are_unique() {
        let n = container_names(vec![
            ("VLC".into(), "org.videolan.vlc-ios".into()),
            ("Notes".into(), "a.notes".into()),
            ("notes".into(), "b.notes".into()),
            ("Media".into(), "x.media".into()),
        ]);
        let names: Vec<&str> = n.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Media (x.media)", "Notes (a.notes)", "notes (b.notes)", "VLC"]);
    }

    /// Browse answers in several frames, with or without file sharing.
    #[tokio::test]
    async fn browses_apps_that_share_files() {
        let (mut client, mut server) = tokio::io::duplex(1 << 16);
        let device = tokio::spawn(async move {
            let req = read_frame(&mut server).await.unwrap();
            assert_eq!(req.get("Command").and_then(|c| c.as_string()), Some("Browse"));
            let app = |id: &str, name: Option<&str>, share: plist::Value| {
                let mut d = plist::Dictionary::new();
                d.insert("CFBundleIdentifier".into(), id.into());
                if let Some(n) = name {
                    d.insert("CFBundleDisplayName".into(), n.into());
                }
                d.insert("UIFileSharingEnabled".into(), share);
                plist::Value::Dictionary(d)
            };
            let frame = |apps: Vec<plist::Value>, status: &str| {
                let mut d = plist::Dictionary::new();
                d.insert("CurrentList".into(), plist::Value::Array(apps));
                d.insert("Status".into(), status.into());
                plist::Value::Dictionary(d)
            };
            let one = frame(
                vec![app("org.vlc", Some("VLC"), true.into()), app("x.no", Some("No"), false.into())],
                "BrowsingProgress",
            );
            write_frame(&mut server, &one).await.unwrap();
            let two = frame(vec![app("y.docs", None, "YES".into())], "BrowsingProgress");
            write_frame(&mut server, &two).await.unwrap();
            let mut done = plist::Dictionary::new();
            done.insert("Status".into(), "Complete".into());
            write_frame(&mut server, &done.into()).await.unwrap();
        });
        let apps = browse(&mut client).await.unwrap();
        device.await.unwrap();
        let app = |name: &str, id: &str, shares| App { name: name.into(), id: id.into(), shares };
        assert_eq!(apps, [app("VLC", "org.vlc", true), app("No", "x.no", false), app("y.docs", "y.docs", true)]);
    }

    /// Against a real device: `NOEXPLORER_TEST_IPHONE=1 cargo test iphone_roundtrip -- --nocapture`
    /// with an unlocked iPhone that trusts this computer. Writes only below `Downloads/nx-test` in
    /// the media area (and removes it).
    #[tokio::test]
    async fn iphone_roundtrip() {
        if std::env::var("NOEXPLORER_TEST_IPHONE").is_err() {
            eprintln!("skipping iphone_roundtrip: NOEXPLORER_TEST_IPHONE not set");
            return;
        }
        let devs = iphone_devices().await.unwrap();
        eprintln!("devices: {devs:?}");
        let ph = IPhone::new(&Profile { name: "test".into(), ..Default::default() });
        assert_eq!(ph.containers().await.unwrap()[0].name, ROOT);
        eprintln!("top: {:?}", ph.list(ROOT, "").await.unwrap().iter().map(|e| &e.key).collect::<Vec<_>>());
        eprintln!("apps: {:?}", ph.list(ROOT, "Apps/").await.unwrap().iter().map(|e| &e.key).collect::<Vec<_>>());
        let top = ph.list(ROOT, "Media/").await.unwrap();
        eprintln!("media: {:?}", top.iter().map(|e| &e.key).collect::<Vec<_>>());

        let dir = std::env::temp_dir().join(format!("nx-iphone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data: Vec<u8> = (0..(3 * 1024 * 1024 + 11)).map(|i| (i * 17 % 251) as u8).collect();
        std::fs::write(dir.join("big.bin"), &data).unwrap();
        let job = Job::untracked("up", "t".into(), "", MEDIA, "");
        let t0 = std::time::Instant::now();
        ph.upload_file(ROOT, "Media/Downloads/nx-test/sub/big.bin", &dir.join("big.bin"), data.len() as u64, &job)
            .await
            .unwrap();
        eprintln!("upload 3 MB: {:?}", t0.elapsed());
        let level = ph.list(ROOT, "Media/Downloads/nx-test/sub/").await.unwrap();
        assert!(level.iter().any(|e| e.key == "Media/Downloads/nx-test/sub/big.bin" && e.size == data.len() as u64));
        assert!(!level.iter().any(|e| e.name.ends_with(".noexplorer-part")));
        let (part, total) =
            ph.read_range(ROOT, "Media/Downloads/nx-test/sub/big.bin", 10, Some(19), 1 << 20).await.unwrap();
        assert_eq!((part.as_slice(), total), (&data[10..20], Some(data.len() as u64)));
        let t0 = std::time::Instant::now();
        ph.download_file(ROOT, "Media/Downloads/nx-test/sub/big.bin", &dir.join("out.bin"), &job).await.unwrap();
        eprintln!("download 3 MB: {:?}", t0.elapsed());
        assert!(std::fs::read(dir.join("out.bin")).unwrap() == data, "download differs");
        ph.transfer_within(ROOT, "Media/Downloads/nx-test/sub/", "Media/Downloads/nx-test/moved/", false, None)
            .await
            .unwrap();
        ph.transfer_within(ROOT, "Media/Downloads/nx-test/moved/", "Media/Downloads/nx-test/copy/", true, None)
            .await
            .unwrap();
        assert_eq!(ph.read_head(ROOT, "Media/Downloads/nx-test/copy/big.bin", 5).await.unwrap(), &data[..5]);
        ph.delete(ROOT, vec!["Media/Downloads/nx-test/".into()], None).await.unwrap();
        assert!(ph.list(ROOT, "Media/Downloads/").await.unwrap().iter().all(|e| e.key != "Media/Downloads/nx-test/"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
