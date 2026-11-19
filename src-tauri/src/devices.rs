//! Connected iPhones / iPads (experimental, ADR-054 update): a watcher on usbmuxd's attach/detach
//! events keeps the list the account menu's "Devices" group shows, and emits `"iphone-devices"`
//! when it changes. Devices need no saved account: a view's account is `iphone:<UDID>`, and
//! `AppState::remote` builds its `Remote::IPhone` on the fly ([`profile`]).
//!
//! - Listed: every connected device (one row per UDID, cable before Wi-Fi), plus devices that
//!   aren't connected but have a backup here (their apps' files stay readable, ADR-055).
//! - The watcher runs on its own thread: usbmuxd's listen stream isn't `Send`. It sleeps without a
//!   timer while the experiment is off (woken when settings are saved), and reconnects every 5 s
//!   (30 s while the window is hidden) when usbmuxd isn't there (Windows without Apple Devices).
//! - "Trusted" changes without an event (the user taps Trust): untrusted devices are checked again
//!   every 3 s (15 s while hidden). With none of them, nothing wakes the thread but usbmuxd (ADR-057).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use futures::StreamExt;
use idevice::IdeviceService;
use idevice::lockdown::LockdownClient;
use idevice::usbmuxd::{Connection, UsbmuxdDevice, UsbmuxdListenEvent};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::config::{Profile, ProviderKind};
use crate::state::AppState;

/// Account ids of devices: `iphone:<UDID>`.
pub const PREFIX: &str = "iphone:";
const ASK: Duration = Duration::from_secs(5);

/// One row of the Devices group. Mirrors `IPhoneDevice` in `src/types.ts`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub udid: String,
    /// "Kris's iPhone"; the class when the device won't say.
    pub name: String,
    /// "iPhone", "iPad"…
    pub class: String,
    /// "iPhone15,2"
    pub model: String,
    /// Connected by cable (else over Wi-Fi, through Finder / Apple Devices sync).
    #[serde(default)]
    pub usb: bool,
    /// The system has a pair record: the device trusts this computer.
    #[serde(default)]
    pub trusted: bool,
    /// Connected now (else only its backup is here).
    #[serde(default)]
    pub connected: bool,
    /// When its backup here was made (Unix millis), if there is one.
    #[serde(default)]
    pub backup: Option<i64>,
}

/// Connected devices, by UDID.
static CONNECTED: LazyLock<Mutex<Vec<DeviceInfo>>> = LazyLock::new(Default::default);
/// Names seen (also from backups), for accounts of devices that aren't connected.
static NAMES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

pub fn is_device(account: &str) -> bool {
    account.starts_with(PREFIX)
}

/// The account behind `iphone:<UDID>`.
pub fn profile(account: &str) -> Profile {
    let udid = account.trim_start_matches(PREFIX);
    let name = NAMES.lock().unwrap().get(udid).cloned().unwrap_or_else(|| "iPhone".into());
    Profile { id: account.into(), kind: ProviderKind::Iphone, name, endpoint: udid.into(), ..Default::default() }
}

/// Connected devices first, then devices with only a backup here.
pub fn list() -> Vec<DeviceInfo> {
    let mut out = CONNECTED.lock().unwrap().clone();
    let backups = crate::iphone_backup::saved_devices();
    for d in &mut out {
        d.backup = backups.iter().find(|b| b.udid == d.udid).and_then(|b| b.backup);
    }
    for b in backups {
        if !out.iter().any(|d| d.udid == b.udid) {
            NAMES.lock().unwrap().entry(b.udid.clone()).or_insert_with(|| b.name.clone());
            out.push(b);
        }
    }
    out
}

fn enabled(app: &AppHandle) -> bool {
    app.state::<AppState>().settings().general.experimental.iphone
}

/// Wakes the watcher: settings saved (the experiment may have been flipped) or the window was
/// shown/hidden (re-check periods change). `notify_one` keeps a permit, so none is lost.
static WAKE: Notify = Notify::const_new();

pub fn settings_changed() {
    WAKE.notify_one();
}

pub fn visibility_changed() {
    WAKE.notify_one();
}

/// How long to wait before re-checking untrusted devices, or before reconnecting to an absent
/// usbmuxd. Longer while the window is hidden: nobody is looking at the Devices group (ADR-057).
fn period(recheck: bool) -> Duration {
    match (recheck, crate::tray::visible()) {
        (true, true) => Duration::from_secs(3),
        (true, false) => Duration::from_secs(15),
        (false, true) => Duration::from_secs(5),
        (false, false) => Duration::from_secs(30),
    }
}

/// Starts the watcher thread. It sleeps on [`WAKE`] while the experiment is off: no polling.
pub fn setup(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new().name("iphone-watch".into()).spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        rt.block_on(async move {
            loop {
                if !enabled(&app) {
                    WAKE.notified().await;
                    continue;
                }
                watch(&app).await;
                publish(&app, vec![]); // usbmuxd went away, or the experiment was turned off
                if enabled(&app) {
                    // usbmuxd isn't there (Windows without Apple Devices): try again later.
                    tokio::select! {
                        _ = tokio::time::sleep(period(false)) => {}
                        _ = WAKE.notified() => {}
                    }
                }
            }
        });
    });
}

/// Name, class and model (no pairing needed), with whether the device trusts this computer.
async fn describe(d: &UsbmuxdDevice) -> DeviceInfo {
    let p = d.to_provider(crate::iphone::addr(), crate::iphone::LABEL);
    let mut l = tokio::time::timeout(ASK, LockdownClient::connect(&p)).await.ok().and_then(Result::ok);
    let mut get = async |key: &str| -> Option<String> {
        let v = tokio::time::timeout(ASK, l.as_mut()?.get_value(Some(key), None)).await.ok()?.ok()?;
        v.as_string().map(str::to_string)
    };
    let model = get("ProductType").await.unwrap_or_default();
    let class = get("DeviceClass").await.unwrap_or_else(|| "iPhone".into());
    let name = get("DeviceName").await.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| class.clone());
    DeviceInfo {
        udid: d.udid.clone(),
        name,
        class,
        model,
        usb: d.connection_type == Connection::Usb,
        trusted: trusted(&d.udid).await,
        connected: true,
        backup: None,
    }
}

async fn trusted(udid: &str) -> bool {
    let Ok(Ok(mut m)) = tokio::time::timeout(ASK, crate::iphone::addr().connect(0)).await else { return false };
    matches!(tokio::time::timeout(ASK, m.get_pair_record(udid)).await, Ok(Ok(_)))
}

/// Rows from what usbmuxd reported (`attached`, by usbmuxd id): one per UDID, cable first.
fn rows(attached: &HashMap<u32, UsbmuxdDevice>, info: &HashMap<String, DeviceInfo>) -> Vec<DeviceInfo> {
    let mut by_udid: Vec<&UsbmuxdDevice> = attached.values().collect();
    by_udid.sort_by_key(|d| (d.udid.clone(), d.connection_type != Connection::Usb));
    by_udid.dedup_by(|a, b| a.udid == b.udid);
    let mut out: Vec<DeviceInfo> = by_udid
        .into_iter()
        .filter_map(|d| {
            info.get(&d.udid).map(|i| DeviceInfo { usb: d.connection_type == Connection::Usb, ..i.clone() })
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn publish(app: &AppHandle, now: Vec<DeviceInfo>) {
    {
        let mut cur = CONNECTED.lock().unwrap();
        if *cur == now {
            return;
        }
        let mut names = NAMES.lock().unwrap();
        for d in &now {
            names.insert(d.udid.clone(), d.name.clone());
        }
        *cur = now;
    }
    let _ = app.emit("iphone-devices", list());
}

/// Follows usbmuxd until it goes away or the experiment is turned off.
async fn watch(app: &AppHandle) {
    let Ok(Ok(mut m)) = tokio::time::timeout(ASK, crate::iphone::addr().connect(0)).await else { return };
    let Ok(mut events) = m.listen().await else { return };
    let mut attached: HashMap<u32, UsbmuxdDevice> = HashMap::new();
    let mut info: HashMap<String, DeviceInfo> = HashMap::new();
    loop {
        // Attach/detach come from usbmuxd's listen stream; a timer runs only while a device still
        // has something to tell (not trusted yet, or no model while locked).
        let recheck = attached.values().any(|d| info.get(&d.udid).is_some_and(|i| !i.trusted || i.model.is_empty()));
        tokio::select! {
            _ = WAKE.notified() => {
                if !enabled(app) {
                    return;
                }
                continue; // the window was shown or hidden: the period changes
            }
            ev = events.next() => match ev {
                // usbmuxd sends an Attached for every device already there, then the changes.
                Some(Ok(UsbmuxdListenEvent::Connected(d))) => {
                    if !info.contains_key(&d.udid) {
                        info.insert(d.udid.clone(), describe(&d).await);
                    }
                    attached.insert(d.device_id, d);
                }
                Some(Ok(UsbmuxdListenEvent::Disconnected(id))) => {
                    if let Some(d) = attached.remove(&id)
                        && !attached.values().any(|x| x.udid == d.udid)
                    {
                        info.remove(&d.udid); // described again next time (it may have been renamed)
                    }
                }
                _ => return, // usbmuxd closed or answered nonsense: start over
            },
            _ = tokio::time::sleep(period(true)), if recheck => {
                // Trust is granted on the device: no event for it. A device plugged in while
                // locked may also have said nothing about itself yet.
                for d in attached.values() {
                    let Some(i) = info.get(&d.udid) else { continue };
                    if !i.trusted || i.model.is_empty() {
                        let again = describe(d).await;
                        info.insert(d.udid.clone(), again);
                    }
                }
            }
        }
        publish(app, rows(&attached, &info));
    }
}

/// The Devices group: connected iPhones / iPads and those with a backup here.
#[tauri::command]
pub fn iphone_connected() -> Vec<DeviceInfo> {
    list()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_row_per_device_cable_first() {
        let dev = |id, udid: &str, usb| UsbmuxdDevice {
            connection_type: if usb { Connection::Usb } else { Connection::Network("10.0.0.2".parse().unwrap()) },
            udid: udid.into(),
            device_id: id,
        };
        let attached = HashMap::from([(1, dev(1, "A", false)), (2, dev(2, "A", true)), (3, dev(3, "B", false))]);
        let info = |udid: &str, name: &str| DeviceInfo {
            udid: udid.into(),
            name: name.into(),
            class: "iPhone".into(),
            model: String::new(),
            usb: false,
            trusted: true,
            connected: true,
            backup: None,
        };
        let infos = HashMap::from([("A".to_string(), info("A", "zed")), ("B".to_string(), info("B", "Anh"))]);
        let r = rows(&attached, &infos);
        assert_eq!(r.iter().map(|d| (d.name.as_str(), d.usb)).collect::<Vec<_>>(), [("Anh", false), ("zed", true)]);
        let p = profile("iphone:A");
        assert_eq!((p.kind, p.endpoint.as_str()), (ProviderKind::Iphone, "A"));
    }
}
