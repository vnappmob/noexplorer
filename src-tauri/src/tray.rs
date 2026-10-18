//! System tray / macOS menu-bar icon and background running.
//!
//! - The tray menu shows live transfer status (fed by `jobs.rs` via [`on_job`]), then the remote
//!   accounts (fed by `config.rs` via [`set_accounts`]): choosing one opens the window on that
//!   account (`"tray-open-account"`). Then Check for Updates / Quit. On macOS the overall % also
//!   appears next to the icon.
//! - Closing the window hides it (when `general.run_in_background` is on, see `lib.rs`);
//!   jobs keep running. On macOS the Dock icon is hidden while the window is hidden.
//! - "Check for Updates…" emits `"tray-check-updates"`; the UI (`updater.ts`) does the check.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::config::Settings;
use crate::util::fmt_bytes;

const MAX_LINES: usize = 5;
const KEEP_FINISHED: usize = 3;

/// One transfer as shown in the tray menu.
#[derive(Clone)]
pub struct Line {
    id: String,
    kind: &'static str,
    label: String,
    bytes_done: u64,
    bytes_total: u64,
    status: &'static str,
}

impl Line {
    fn text(&self) -> String {
        let arrow = match self.kind {
            "up" => "⬆",
            "delete" => "🗑",
            "copy" => "⇄",
            "move" => "↪",
            "dup" => "⧉",
            _ => "⬇",
        };
        let label: String = if self.label.chars().count() > 34 {
            format!("{}…", self.label.chars().take(33).collect::<String>())
        } else {
            self.label.clone()
        };
        match self.status {
            "running" if self.bytes_total > 0 => format!(
                "{arrow} {label} — {}% ({} of {})",
                percent(self.bytes_done, self.bytes_total).unwrap_or(0),
                fmt_bytes(self.bytes_done),
                fmt_bytes(self.bytes_total)
            ),
            "running" => format!("{arrow} {label} — preparing…"),
            "done" => format!("✓ {label} — done"),
            "cancelled" => format!("⨯ {label} — cancelled"),
            _ => format!("⚠ {label} — failed"),
        }
    }
}

/// A remote account in the menu: (id, name, active).
type Account = (String, String, bool);

#[derive(Default)]
pub struct TrayState {
    lines: Mutex<Vec<Line>>,
    accounts: Mutex<Vec<Account>>,
    /// Ids currently in the menu; the menu is rebuilt only when this set changes.
    shown: Mutex<Vec<String>>,
    status_item: Mutex<Option<MenuItem<Wry>>>,
    line_items: Mutex<Vec<MenuItem<Wry>>>,
}

fn tray(app: &AppHandle) -> Option<TrayIcon> {
    app.tray_by_id("main")
}

pub fn show_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    // Menu-bar-only while hidden: no Dock icon, no app switcher entry.
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

pub fn setup(app: &AppHandle, settings: &Settings) -> tauri::Result<()> {
    app.manage(TrayState::default());
    *app.state::<TrayState>().accounts.lock().unwrap() = accounts_of(settings);
    let menu = build_menu(app, &[])?;
    #[cfg(target_os = "macos")]
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = app.default_window_icon().cloned().unwrap_or(Image::from_bytes(include_bytes!("../icons/32x32.png"))?);
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("NoExplorer")
        .menu(&menu)
        // Windows/Linux convention: left click opens the window, right click the menu.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            id if id.starts_with("acct:") => {
                show_main(app);
                let _ = app.emit("tray-open-account", &id[5..]);
                // A check item toggles itself when clicked: redraw so only the active one is checked
                // (the UI's switch then updates it through set_accounts).
                rebuild(app);
            }
            "add-account" => {
                show_main(app);
                let _ = app.emit("tray-add-account", ());
            }
            "updates" => {
                show_main(app);
                let _ = app.emit("tray-check-updates", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|icon, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev
                && !cfg!(target_os = "macos")
            {
                show_main(icon.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn build_menu(app: &AppHandle, lines: &[Line]) -> tauri::Result<Menu<Wry>> {
    let st = app.state::<TrayState>();
    let status = MenuItem::with_id(app, "status", summary(lines), false, None::<&str>)?;
    let line_items: Vec<MenuItem<Wry>> = lines
        .iter()
        .map(|l| MenuItem::with_id(app, format!("job:{}", l.id), l.text(), false, None::<&str>))
        .collect::<tauri::Result<_>>()?;
    let accounts = st.accounts.lock().unwrap().clone();
    let header = MenuItem::with_id(app, "accounts", "Accounts", false, None::<&str>)?;
    let account_items: Vec<CheckMenuItem<Wry>> = accounts
        .iter()
        .map(|(id, name, active)| CheckMenuItem::with_id(app, format!("acct:{id}"), name, true, *active, None::<&str>))
        .collect::<tauri::Result<_>>()?;
    let add = MenuItem::with_id(app, "add-account", "Add Account…", true, None::<&str>)?;
    let updates = MenuItem::with_id(app, "updates", "Check for Updates…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit NoExplorer", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;

    let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![&status];
    items.extend(line_items.iter().map(|i| i as &dyn IsMenuItem<Wry>));
    items.push(&sep1);
    if account_items.is_empty() {
        items.push(&add);
    } else {
        items.push(&header);
        items.extend(account_items.iter().map(|i| i as &dyn IsMenuItem<Wry>));
    }
    items.extend([&sep2 as &dyn IsMenuItem<Wry>, &updates, &sep3, &quit]);
    let menu = Menu::with_items(app, &items)?;

    *st.status_item.lock().unwrap() = Some(status);
    *st.line_items.lock().unwrap() = line_items;
    *st.shown.lock().unwrap() = lines.iter().map(|l| l.id.clone()).collect();
    Ok(menu)
}

/// Whole-number percentage, or None when the total isn't known yet.
fn percent(done: u64, total: u64) -> Option<u64> {
    (done * 100).checked_div(total)
}

fn summary(lines: &[Line]) -> String {
    let running: Vec<&Line> = lines.iter().filter(|l| l.status == "running").collect();
    if running.is_empty() {
        return "No active transfers".into();
    }
    let (done, total) = running.iter().fold((0, 0), |(d, t), l| (d + l.bytes_done, t + l.bytes_total));
    let pct = percent(done, total).map(|p| format!(" · {p}%")).unwrap_or_default();
    format!("{} active transfer{}{pct}", running.len(), if running.len() == 1 { "" } else { "s" })
}

/// Called by `jobs.rs` on every progress tick and on completion.
#[allow(clippy::too_many_arguments)]
pub fn on_job(
    app: &AppHandle,
    id: &str,
    kind: &'static str,
    label: &str,
    bytes_done: u64,
    bytes_total: u64,
    status: &'static str,
) {
    let Some(st) = app.try_state::<TrayState>() else { return };
    let lines = {
        let mut lines = st.lines.lock().unwrap();
        let line = Line { id: id.into(), kind, label: label.into(), bytes_done, bytes_total, status };
        match lines.iter_mut().find(|l| l.id == id) {
            Some(l) => *l = line,
            None => lines.insert(0, line),
        }
        // Running first (newest first), then the most recent finished ones.
        let (mut keep, finished): (Vec<Line>, Vec<Line>) = lines.drain(..).partition(|l| l.status == "running");
        keep.extend(finished.into_iter().take(KEEP_FINISHED));
        keep.truncate(MAX_LINES);
        *lines = keep;
        lines.clone()
    };
    refresh(app, &st, &lines);
}

fn refresh(app: &AppHandle, st: &TrayState, lines: &[Line]) {
    let Some(tray) = tray(app) else { return };
    let ids: Vec<String> = lines.iter().map(|l| l.id.clone()).collect();
    if *st.shown.lock().unwrap() != ids {
        if let Ok(menu) = build_menu(app, lines) {
            let _ = tray.set_menu(Some(menu));
        }
    } else {
        // Same rows: update texts in place (doesn't close an open menu).
        if let Some(s) = st.status_item.lock().unwrap().as_ref() {
            let _ = s.set_text(summary(lines));
        }
        for (item, line) in st.line_items.lock().unwrap().iter().zip(lines) {
            let _ = item.set_text(line.text());
        }
    }
    let _ = tray.set_tooltip(Some(format!("NoExplorer — {}", summary(lines))));
    // macOS shows the title next to the menu-bar icon: a compact overall %.
    #[cfg(target_os = "macos")]
    {
        let running: Vec<&Line> = lines.iter().filter(|l| l.status == "running").collect();
        let (d, t) = running.iter().fold((0, 0), |(d, t), l| (d + l.bytes_done, t + l.bytes_total));
        let title = percent(d, t).filter(|_| !running.is_empty()).map(|p| format!("{p}%"));
        let _ = tray.set_title(title);
    }
}

fn accounts_of(s: &Settings) -> Vec<Account> {
    let active = s.active_profile().map(|p| p.id.clone()).unwrap_or_default();
    s.profiles.iter().map(|p| (p.id.clone(), p.name.clone(), p.id == active)).collect()
}

/// Called by `config.rs` whenever accounts change (add, rename, remove, switch).
pub fn set_accounts(app: &AppHandle, s: &Settings) {
    let Some(st) = app.try_state::<TrayState>() else { return };
    let accounts = accounts_of(s);
    if *st.accounts.lock().unwrap() == accounts {
        return;
    }
    *st.accounts.lock().unwrap() = accounts;
    rebuild(app);
}

fn rebuild(app: &AppHandle) {
    let Some(st) = app.try_state::<TrayState>() else { return };
    let lines = st.lines.lock().unwrap().clone();
    if let (Some(tray), Ok(menu)) = (tray(app), build_menu(app, &lines)) {
        let _ = tray.set_menu(Some(menu));
    }
}
