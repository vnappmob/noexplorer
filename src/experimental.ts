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
    key: "bucket_policy",
    title: "Bucket access policies",
    desc: "See and change who can read an S3 bucket without a key: private, public read for the whole bucket or some folders, or your own policy JSON. Plus “Copy public link” for files. In the bucket menu, the right-click menu and ⌘K.",
  },
  {
    key: "remote_sizes",
    title: "Remote folder sizes",
    desc: "Count the size of every folder you open on S3 or Azure, in the background (at most 200,000 files per folder; bigger ones show “≥”). Off, sizes come from Σ only, and are remembered either way.",
  },
  {
    key: "box_beta",
    title: "Box accounts",
    desc: "Box in the account form, with your own Box app. It works against our test double but hasn't been tried on a real Box account yet.",
  },
];

/** Fills the Experimental tab (index.html #exp-form). */
export function renderExperimental() {
  const g = app.settings.general;
  $("exp-list").innerHTML = EXPERIMENTS.map(
    (e) => `<label class="set-row"><span><b>${esc(e.title)}</b><small>${esc(e.desc)}</small></span>
      <input type="checkbox" class="switch" name="${e.key}"${g.experimental?.[e.key] ? " checked" : ""} /></label>`,
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
