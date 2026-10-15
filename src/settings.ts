// Settings dialog (connection form) and the bucket picker.

import { invoke } from "@tauri-apps/api/core";
import { remote } from "./panes";
import { app } from "./state";
import type { Config } from "./types";
import { $, esc, fail, plural, store } from "./util";

const form = () => $<HTMLFormElement>("settings-form");
const FIELDS = ["endpoint", "region", "access_key", "secret_key", "bucket"] as const;

function readForm(): Config {
  const f = new FormData(form());
  const v = (k: string) => String(f.get(k) ?? "").trim();
  return {
    endpoint: v("endpoint"),
    region: v("region"),
    access_key: v("access_key"),
    secret_key: v("secret_key"),
    bucket: v("bucket"),
    path_style: f.get("path_style") === "on",
    download_dir: app.cfg?.download_dir ?? "",
  };
}

export function openSettings() {
  const f = form();
  const c = app.cfg ?? ({ region: "us-east-1", path_style: true } as Config);
  for (const k of FIELDS) (f.elements.namedItem(k) as HTMLInputElement).value = c[k] ?? "";
  (f.elements.namedItem("path_style") as HTMLInputElement).checked = c.path_style ?? true;
  setMsg("");
  $<HTMLDialogElement>("settings").showModal();
}

function setMsg(text: string, cls = "") {
  const m = $("settings-msg");
  m.textContent = text;
  m.className = cls;
}

async function testSettings(): Promise<boolean> {
  setMsg("Testing…");
  try {
    const b = await invoke<string[]>("test_config", { cfg: readForm() });
    setMsg(`✓ Connected · ${plural(b.length, "bucket")}: ${b.slice(0, 8).join(", ")}${b.length > 8 ? "…" : ""}`, "ok");
    return true;
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
    return false;
  }
}

/** Save = test first, then persist, then reload buckets. */
async function saveSettings(ev: Event) {
  ev.preventDefault();
  if (!form().reportValidity()) return;
  if (!(await testSettings())) return;
  const c = readForm();
  try {
    await invoke("save_config", { cfg: c });
    if (!app.cfg || app.cfg.endpoint !== c.endpoint || app.cfg.access_key !== c.access_key) {
      // Different server/account: forget the last bucket/prefix.
      store.set("bucket", c.bucket);
      store.set("prefix", "");
    }
    app.cfg = c;
    $<HTMLDialogElement>("settings").close();
    await loadBuckets();
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
  }
}

/** Fills the bucket picker and restores the last bucket + prefix when possible. */
export async function loadBuckets() {
  const sel = $<HTMLSelectElement>("bucket");
  let names: string[] = [];
  try {
    names = await invoke<string[]>("list_buckets");
  } catch (e) {
    fail(e);
  }
  const last = store.get("bucket");
  const cfgBucket = app.cfg?.bucket ?? "";
  app.bucket = names.includes(last ?? "") ? last! : names.includes(cfgBucket) ? cfgBucket : (names[0] ?? "");
  sel.innerHTML = names.map((n) => `<option ${n === app.bucket ? "selected" : ""}>${esc(n)}</option>`).join("");
  remote.cwd = app.bucket && app.bucket === last ? (store.get("prefix") ?? "") : "";
  await remote.load();
}

export function setupSettings() {
  form().addEventListener("submit", saveSettings);
  $("settings-test").onclick = testSettings;
  $("settings-cancel").onclick = () => $<HTMLDialogElement>("settings").close();
  $("btn-settings").onclick = openSettings;
  $<HTMLSelectElement>("bucket").addEventListener("change", (ev) => {
    app.bucket = (ev.target as HTMLSelectElement).value;
    remote.go("");
  });
}
