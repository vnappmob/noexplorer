// Settings → Experimental: features that ship off by default. Each one checks `exp("key")`
// (state.ts) where it hooks in, and listens for the "experimental" window event to react when the
// user flips it. The backend mirror is `Experimental` in config.rs.

import { invoke } from "@tauri-apps/api/core";
import { app } from "./state";
import type { Experimental, Settings } from "./types";
import { $, esc, fail } from "./util";

export const EXPERIMENTS: { key: keyof Experimental; title: string; desc: string }[] = [
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
  {
    key: "providers",
    title: "More storage types",
    desc: "SFTP, Google Cloud Storage and Azure Blob Storage in the account form.",
  },
];

/** Fills the Experimental tab (index.html #exp-form). */
export function renderExperimental() {
  const g = app.settings.general;
  $("exp-list").innerHTML = EXPERIMENTS.map(
    (e) => `<label class="check"><input type="checkbox" name="${e.key}"${g.experimental?.[e.key] ? " checked" : ""} />
      <span><b>${esc(e.title)}</b><br /><small>${esc(e.desc)}</small></span></label>`,
  ).join("");
}

async function save() {
  const form = $<HTMLFormElement>("exp-form");
  const experimental = Object.fromEntries(
    EXPERIMENTS.map((e) => [e.key, (form.elements.namedItem(e.key) as HTMLInputElement).checked]),
  ) as Experimental;
  try {
    app.settings = await invoke<Settings>("save_general", {
      general: { ...app.settings.general, experimental },
    });
    window.dispatchEvent(new Event("experimental"));
  } catch (e) {
    fail(e);
    renderExperimental();
  }
}

export function setupExperimental() {
  $("exp-form").addEventListener("change", save);
}
