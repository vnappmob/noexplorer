// Auto-update (OTA). Installed apps poll `latest.json` on GitHub Releases (see
// tauri.conf.json → plugins.updater); bundles are verified against the embedded public key.
// Flow: check at startup (+ every 6 h) when enabled → banner → "Install & restart".
// Manual checks come from Settings (General/About) and the tray menu ("tray-check-updates").

import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { activeJobCount } from "./jobs";
import { app } from "./state";
import { $, fail, fmtSize, plural, toast } from "./util";

const EVERY = 6 * 60 * 60 * 1000;
let pending: Update | null = null;
let busy = false;

/** Checks GitHub for a newer version. `manual` = user asked: report "up to date" and errors. */
export async function checkForUpdates(manual = false) {
  if (busy) return;
  busy = true;
  try {
    const u = await check();
    if (u) showBanner(u);
    else if (manual) toast("NoExplorer is up to date");
  } catch (e) {
    if (manual) fail(`Update check failed: ${e}`);
    else console.warn("update check failed", e);
  } finally {
    busy = false;
  }
}

function showBanner(u: Update) {
  pending = u;
  $("upd-text").textContent = `NoExplorer ${u.version} is available (you have ${u.currentVersion}).`;
  $<HTMLButtonElement>("upd-install").disabled = false;
  $("upd-install").textContent = "Install & restart";
  $("update-bar").hidden = false;
}

async function install() {
  const u = pending;
  if (!u) return;
  const running = activeJobCount();
  if (running) {
    const ok = await ask(`${plural(running, "transfer")} still running. Restarting will stop them. Update anyway?`, {
      title: "Update NoExplorer",
      kind: "warning",
      okLabel: "Update now",
      cancelLabel: "Later",
    });
    if (!ok) return;
  }
  const btn = $<HTMLButtonElement>("upd-install");
  btn.disabled = true;
  let total = 0;
  let got = 0;
  try {
    await u.downloadAndInstall((ev) => {
      if (ev.event === "Started") total = ev.data.contentLength ?? 0;
      if (ev.event === "Progress") {
        got += ev.data.chunkLength;
        btn.textContent = total ? `Downloading… ${Math.floor((got / total) * 100)}%` : `Downloading… ${fmtSize(got)}`;
      }
      if (ev.event === "Finished") btn.textContent = "Installing…";
    });
    // Relaunch reuses this run's arguments; a login start (--hidden) would come back hidden (tray.rs).
    await invoke("show_after_relaunch").catch(() => {});
    await relaunch();
  } catch (e) {
    btn.disabled = false;
    btn.textContent = "Install & restart";
    fail(`Update failed: ${e}`);
  }
}

export async function setupUpdater() {
  $("upd-install").onclick = install;
  $("upd-later").onclick = () => ($("update-bar").hidden = true);
  $("upd-notes").onclick = () =>
    pending && openUrl(`https://github.com/vnappmob/noexplorer/releases/tag/v${pending.version}`).catch(fail);
  await listen("tray-check-updates", () => checkForUpdates(true));
  const auto = () => app.settings.general.check_updates && checkForUpdates(false);
  setTimeout(auto, 4000); // let the UI settle first
  setInterval(auto, EVERY);
}
