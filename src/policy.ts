// Bucket access policies (experimental, ADR-047): who may read a bucket's files without a key.
// Three presets cover the common cases (private, public-read for the whole bucket, public-read for
// some folders) and are written as a standard S3 bucket policy; anything else is edited as JSON.
// The backend (`bucket_policy` / `set_bucket_policy`, s3.rs) only stores and fetches it, so the
// server's own rules (AWS Block Public Access, unsupported providers) come back as its errors.

import { invoke } from "@tauri-apps/api/core";
import type { Pane } from "./pane";
import { isS3 } from "./providers";
import { exp } from "./state";
import type { Item } from "./types";
import { esc, fail, toast } from "./util";

type Mode = "private" | "bucket" | "folders" | "custom";

/** Sid of the statement NoExplorer writes, so its own policies are recognized again. */
const SID = "NoExplorerPublicRead";

/** Policy features are offered for S3 views with the experiment on. */
export const canManagePolicy = (p: Pane) => exp("bucket_policy") && p.isRemote && !!p.bucket && isS3(p.profile());

const folderOf = (prefix: string) => prefix.replace(/^\/+/, "").replace(/\/*$/, "/");

/** A public-read policy for the whole bucket (no folders) or for these folders. */
function publicRead(bucket: string, folders: string[]) {
  const resources = folders.length
    ? folders.map((f) => `arn:aws:s3:::${bucket}/${folderOf(f)}*`)
    : [`arn:aws:s3:::${bucket}/*`];
  const policy = {
    Version: "2012-10-17",
    Statement: [{ Sid: SID, Effect: "Allow", Principal: "*", Action: ["s3:GetObject"], Resource: resources }],
  };
  return JSON.stringify(policy, null, 2);
}

const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : v === undefined ? [] : [v]);

/** Which preset a policy is, if any: one Allow statement giving everyone s3:GetObject on prefixes. */
function detect(bucket: string, json: string | null): { mode: Mode; folders: string[] } {
  if (!json) return { mode: "private", folders: [] };
  try {
    const p = JSON.parse(json);
    const st = list(p.Statement) as Record<string, unknown>[];
    if (st.length !== 1) return { mode: "custom", folders: [] };
    const s = st[0];
    const principal = s.Principal as unknown;
    const everyone =
      principal === "*" || list((principal as { AWS?: unknown } | undefined)?.AWS).every((a) => a === "*");
    const actions = list(s.Action);
    if (s.Effect !== "Allow" || s.Condition || !everyone || actions.length !== 1 || actions[0] !== "s3:GetObject")
      return { mode: "custom", folders: [] };
    const head = `arn:aws:s3:::${bucket}/`;
    const folders: string[] = [];
    for (const r of list(s.Resource)) {
      if (typeof r !== "string" || !r.startsWith(head) || !r.endsWith("*")) return { mode: "custom", folders: [] };
      const f = r.slice(head.length, -1);
      if (f && !f.endsWith("/")) return { mode: "custom", folders: [] }; // a name prefix, not a folder
      folders.push(f);
    }
    return folders.includes("") ? { mode: "bucket", folders: [] } : { mode: "folders", folders };
  } catch {
    return { mode: "custom", folders: [] };
  }
}

// ---------- the dialog ----------

let pane: Pane | null = null;
let saved: string | null = null;

function dlg() {
  let d = document.getElementById("policy") as HTMLDialogElement | null;
  if (d) return d;
  d = document.createElement("dialog");
  d.id = "policy";
  d.setAttribute("aria-label", "Access policy");
  d.innerHTML = `<form method="dialog" class="pol-form">
    <header><b>Access policy</b><small id="pol-bucket"></small></header>
    <p class="pol-intro">Who can read this bucket's files <b>without a key</b>. Your access keys keep full access whatever you pick.</p>
    <div class="pol-opts" role="radiogroup" aria-label="Access">
      <label class="pol-opt"><input type="radio" name="pol" value="private" /><span><b>Private</b><small>No policy: only access keys can read or write.</small></span></label>
      <label class="pol-opt"><input type="radio" name="pol" value="bucket" /><span><b>Public read: whole bucket</b><small>Anyone with a file's address can download it.</small></span></label>
      <label class="pol-opt"><input type="radio" name="pol" value="folders" /><span><b>Public read: some folders</b><small>Only files in these folders (and below) are public.</small></span></label>
      <textarea id="pol-folders" rows="3" spellcheck="false" placeholder="One folder per line, e.g. photos/ or website/assets/"></textarea>
      <label class="pol-opt"><input type="radio" name="pol" value="custom" /><span><b>Custom policy</b><small>Edit the JSON yourself: listing, uploads, conditions, other principals…</small></span></label>
    </div>
    <details id="pol-json-box"><summary>Policy JSON</summary><textarea id="pol-json" rows="12" spellcheck="false"></textarea></details>
    <p id="pol-public" class="callout" hidden></p>
    <p id="pol-msg" class="pol-msg"></p>
    <div class="buttons">
      <span class="spacer"></span>
      <button type="button" id="pol-cancel">Cancel</button>
      <button type="submit" class="primary" id="pol-save">Save</button>
    </div>
  </form>`;
  document.body.append(d);
  const q = <T extends HTMLElement>(id: string) => d!.querySelector<T>(`#${id}`)!;
  d.querySelector(".pol-opts")!.addEventListener("change", (ev) => {
    if ((ev.target as HTMLInputElement).name === "pol") fromPreset();
  });
  q("pol-folders").addEventListener("input", () => {
    setMode("folders");
    fromPreset();
  });
  q("pol-json").addEventListener("input", () => {
    setMode("custom");
    sync();
  });
  q("pol-cancel").onclick = () => d!.close();
  d.querySelector("form")!.addEventListener("submit", (ev) => {
    ev.preventDefault();
    save();
  });
  return d;
}

const el = <T extends HTMLElement>(id: string) => dlg().querySelector<T>(`#${id}`)!;
const mode = () => (dlg().querySelector<HTMLInputElement>('input[name="pol"]:checked')?.value ?? "private") as Mode;
const setMode = (m: Mode) => (dlg().querySelector<HTMLInputElement>(`input[name="pol"][value="${m}"]`)!.checked = true);
const folders = () =>
  el<HTMLTextAreaElement>("pol-folders")
    .value.split("\n")
    .map((f) => f.trim())
    .filter(Boolean);

/** A preset was picked (or its folders edited): write its JSON. */
function fromPreset() {
  if (!pane) return;
  const m = mode();
  const json = el<HTMLTextAreaElement>("pol-json");
  if (m === "private") json.value = "";
  if (m === "bucket") json.value = publicRead(pane.bucket, []);
  if (m === "folders") json.value = publicRead(pane.bucket, folders());
  if (m === "custom") {
    if (!json.value.trim()) json.value = saved ?? publicRead(pane.bucket, []);
    (el("pol-json-box") as HTMLDetailsElement).open = true;
  }
  sync();
}

/** Folder box, warning and the example public address follow the choice. */
async function sync() {
  if (!pane) return;
  const m = mode();
  el("pol-folders").hidden = m !== "folders";
  el<HTMLTextAreaElement>("pol-json").readOnly = m !== "custom";
  const pub = el("pol-public");
  pub.hidden = m === "private" || m === "custom";
  el("pol-msg").textContent = "";
  if (pub.hidden) return;
  const p = pane;
  const where = m === "bucket" ? "" : folderOf(folders()[0] ?? "folder/");
  try {
    const url = await invoke<string>("public_url", { ...p.where(), key: `${where}file.jpg` });
    if (p !== pane || mode() !== m) return;
    pub.innerHTML = `Anyone who has a file's address can download it, e.g. <code>${esc(url)}</code>. Nobody can list the files unless you allow it in a custom policy.`;
  } catch {
    pub.textContent = "Anyone who has a file's address can download it.";
  }
}

async function save() {
  if (!pane) return;
  const m = mode();
  const json = el<HTMLTextAreaElement>("pol-json").value.trim();
  if (m === "folders" && !folders().length) return void (el("pol-msg").textContent = "Enter at least one folder.");
  if (m !== "private") {
    try {
      JSON.parse(json);
    } catch (e) {
      el("pol-msg").textContent = `✗ The policy isn't valid JSON: ${(e as Error).message}`;
      return;
    }
  }
  const btn = el<HTMLButtonElement>("pol-save");
  btn.disabled = true;
  el("pol-msg").textContent = "Saving…";
  try {
    await invoke("set_bucket_policy", { ...pane.where(), policy: m === "private" ? null : json });
    dlg().close();
    toast(m === "private" ? `“${pane.bucket}” is private` : `Policy saved for “${pane.bucket}”`);
  } catch (e) {
    el("pol-msg").textContent = `✗ ${e}`;
  } finally {
    btn.disabled = false;
  }
}

/** Opens the policy of `p`'s bucket. `folder` = a folder to suggest for "Public read: some folders". */
export async function openPolicy(p: Pane, folder = p.cwd) {
  const d = dlg();
  pane = p;
  el("pol-bucket").textContent = p.bucket;
  el("pol-msg").textContent = "Loading…";
  el<HTMLButtonElement>("pol-save").disabled = true;
  if (!d.open) d.showModal();
  try {
    saved = await invoke<string | null>("bucket_policy", p.where());
  } catch (e) {
    el("pol-msg").textContent = `✗ ${e}`;
    return;
  }
  if (pane !== p) return;
  const found = detect(p.bucket, saved);
  setMode(found.mode);
  el<HTMLTextAreaElement>("pol-json").value = saved ?? "";
  el<HTMLTextAreaElement>("pol-folders").value = (
    found.folders.length ? found.folders : folder ? [folderOf(folder)] : []
  ).join("\n");
  (el("pol-json-box") as HTMLDetailsElement).open = found.mode === "custom";
  el<HTMLButtonElement>("pol-save").disabled = false;
  await sync();
}

/** Copies a file's unsigned address (it works only when a policy makes the file public). */
export async function copyPublicLink(p: Pane, e: Item) {
  try {
    await navigator.clipboard.writeText(await invoke<string>("public_url", { ...p.where(), key: e.key }));
    toast("Public link copied (it opens only if a policy makes this file public)");
  } catch (err) {
    fail(err);
  }
}
