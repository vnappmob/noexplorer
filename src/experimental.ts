// Settings → Experimental: features that ship off by default. Each one checks `exp("key")`
// (state.ts) where it hooks in, and listens for the "experimental" window event to react when the
// user flips it. The backend mirror is `Experimental` in config.rs.

import { invoke } from "@tauri-apps/api/core";
import { app } from "./state";
import type { Experimental, Settings } from "./types";
import { $, esc, fail } from "./util";

export const EXPERIMENTS: { key: keyof Experimental; title: string; desc: string }[] = [
  {
    key: "file_ops",
    title: "Rename, move and duplicate",
    desc: "F2 renames, plus Move to… and Duplicate in the right-click menu, on this computer and remote. ⌘Z undoes the last one.",
  },
  {
    key: "resume",
    title: "Resumable transfers",
    desc: "Interrupted uploads and downloads continue where they stopped, failed parts retry on their own, and transfers can be paused. Optional speed limit below.",
  },
  {
    key: "self_signed",
    title: "Self-signed HTTPS",
    desc: "An account can trust its server's own certificate (common on NAS boxes). You confirm the certificate once, per account.",
  },
  {
    key: "backup_pairs",
    title: "Backup pairs",
    desc: "Save “this folder ↔ that remote folder” and back it up in one click: Compare, then upload what's missing.",
  },
  { key: "grid", title: "Thumbnail grid", desc: "A grid view with image and video thumbnails, next to the list." },
  {
    key: "tabs",
    title: "Tabs, history and favorites",
    desc: "Back and forward (⌘[ ⌘]), tabs in each pane (⌘T) and a favorites list.",
  },
  {
    key: "providers",
    title: "More storage types",
    desc: "SFTP, Google Cloud Storage and Azure Blob Storage in the account form.",
  },
  {
    key: "native_look",
    title: "Native window look",
    desc: "Windows 11 Mica and macOS translucency behind the panes.",
  },
];

/** Fills the Experimental tab (index.html #exp-form). */
export function renderExperimental() {
  const g = app.settings.general;
  $("exp-list").innerHTML = EXPERIMENTS.map(
    (e) => `<label class="check"><input type="checkbox" name="${e.key}"${g.experimental?.[e.key] ? " checked" : ""} />
      <span><b>${esc(e.title)}</b><br /><small>${esc(e.desc)}</small></span></label>`,
  ).join("");
  $<HTMLInputElement>("speed-limit").value = g.speed_limit_kbps ? String(g.speed_limit_kbps) : "";
  $("speed-row").hidden = !g.experimental?.resume;
}

async function save() {
  const form = $<HTMLFormElement>("exp-form");
  const experimental = Object.fromEntries(
    EXPERIMENTS.map((e) => [e.key, (form.elements.namedItem(e.key) as HTMLInputElement).checked]),
  ) as Experimental;
  const kbps = Math.max(0, Math.floor(Number($<HTMLInputElement>("speed-limit").value) || 0));
  try {
    app.settings = await invoke<Settings>("save_general", {
      general: { ...app.settings.general, experimental, speed_limit_kbps: kbps },
    });
    $("speed-row").hidden = !experimental.resume;
    window.dispatchEvent(new Event("experimental"));
  } catch (e) {
    fail(e);
    renderExperimental();
  }
}

export function setupExperimental() {
  $("exp-form").addEventListener("change", save);
}
