// Settings dialog (Accounts / General / Appearance / About tabs), the account picker and the
// bucket picker in the remote pane header.

import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { appearance, setTextSize, setTheme, type Theme } from "./appearance";
import { remote } from "./panes";
import { describeAccount, guessProvider, PRESETS, type ProviderId } from "./providers";
import { activeProfile, app, profileKey } from "./state";
import type { General, Profile, Settings } from "./types";
import { checkForUpdates } from "./updater";
import { $, esc, fail, plural, store } from "./util";

const dialog = () => $<HTMLDialogElement>("settings");
const form = () => $<HTMLFormElement>("acct-form");
const FIELDS = ["name", "endpoint", "region", "access_key", "secret_key", "bucket"] as const;
const ADD = "__add__";

/** Id of the account in the form; "" = a new, unsaved account. */
let editing = "";

// ---------- dialog ----------

export type Tab = "accounts" | "general" | "appearance" | "about";

export function openSettings(tab: Tab = "accounts", profileId?: string) {
  showTab(tab);
  const id = profileId ?? activeProfile()?.id ?? "";
  editAccount(app.settings.profiles.find((p) => p.id === id) ?? null);
  if (!dialog().open) dialog().showModal();
}

function showTab(tab: string) {
  dialog()
    .querySelectorAll<HTMLElement>(".tabs [data-tab]")
    .forEach((b) => b.classList.toggle("on", b.dataset.tab === tab));
  dialog()
    .querySelectorAll<HTMLElement>("section.tab")
    .forEach((s) => (s.hidden = s.dataset.tab !== tab));
  if (tab === "appearance") syncAppearanceControls();
  if (tab === "general") syncGeneralControls();
  if (tab === "about") getVersion().then((v) => ($("about-version").textContent = v));
}

// ---------- accounts tab ----------

function renderAccountList() {
  const active = activeProfile()?.id;
  $("acct-items").innerHTML = app.settings.profiles
    .map(
      (
        p,
      ) => `<button type="button" class="acct${p.id === editing ? " on" : ""}" data-id="${esc(p.id)}" title="${esc(p.name)}">
        ${p.id === active ? '<span class="dot" title="In use">●</span> ' : ""}${esc(p.name)}
        <small title="${esc(p.endpoint)}">${esc(describeAccount(p))}</small></button>`,
    )
    .join("");
  if (!editing)
    $("acct-items").insertAdjacentHTML("beforeend", `<button type="button" class="acct on">New account</button>`);
}

/** Loads `p` into the form (null = blank form for a new account). */
function editAccount(p: Profile | null) {
  editing = p?.id ?? "";
  const f = form();
  const v: Partial<Profile> = p ?? { region: "us-east-1", path_style: true };
  for (const k of FIELDS) (f.elements.namedItem(k) as HTMLInputElement).value = String(v[k] ?? "");
  (f.elements.namedItem("path_style") as HTMLInputElement).checked = v.path_style ?? true;
  const provider = p ? guessProvider(p) : "custom";
  $<HTMLSelectElement>("acct-provider").value = provider;
  applyPreset(provider, false);
  $<HTMLButtonElement>("acct-delete").hidden = !p;
  setMsg("");
  renderAccountList();
  (f.elements.namedItem(p ? "name" : "endpoint") as HTMLInputElement).focus();
}

const input = (name: string) => form().elements.namedItem(name) as HTMLInputElement;

/** Updates hints/placeholders for a provider. `fill` = the user just picked it: also reset
 * fields that belong to another provider and set the provider's defaults. */
function applyPreset(id: ProviderId, fill: boolean) {
  const pr = PRESETS[id];
  $("acct-hint").textContent = pr.hint;
  const endpoint = input("endpoint");
  endpoint.placeholder = pr.endpointPlaceholder;
  endpoint.required = pr.endpointRequired;
  input("region").placeholder = pr.regionPlaceholder;
  if (!fill) return;
  if (guessProvider({ endpoint: endpoint.value }) !== id) endpoint.value = "";
  input("region").value = pr.region;
  input("path_style").checked = pr.pathStyle;
  (pr.endpointRequired ? endpoint : input("access_key")).focus();
}

function readForm(): Profile {
  const f = new FormData(form());
  const v = (k: string) => String(f.get(k) ?? "").trim();
  return {
    id: editing,
    name: v("name"),
    endpoint: v("endpoint"),
    region: v("region"),
    access_key: v("access_key"),
    secret_key: v("secret_key"),
    bucket: v("bucket"),
    path_style: f.get("path_style") === "on",
  };
}

function setMsg(text: string, cls = "") {
  const m = $("acct-msg");
  m.textContent = text;
  m.className = cls;
}

async function testAccount(): Promise<boolean> {
  if (!form().reportValidity()) return false;
  setMsg("Testing…");
  try {
    const b = await invoke<string[]>("test_profile", { profile: readForm() });
    setMsg(`✓ Connected · ${plural(b.length, "bucket")}: ${b.slice(0, 8).join(", ")}${b.length > 8 ? "…" : ""}`, "ok");
    return true;
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
    return false;
  }
}

/** Save = test first, then persist and switch the remote pane to this account. */
async function saveAccount(ev: Event) {
  ev.preventDefault();
  if (!(await testAccount())) return;
  try {
    const s = await invoke<Settings>("save_profile", { profile: readForm() });
    await applySettings(s);
    dialog().close();
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
  }
}

async function deleteAccount() {
  const p = app.settings.profiles.find((x) => x.id === editing);
  if (!p) return;
  const ok = await ask(`Remove the account "${p.name}" from NoExplorer?\nNothing on the server is deleted.`, {
    title: "Remove account",
    kind: "warning",
    okLabel: "Remove",
    cancelLabel: "Cancel",
  });
  if (!ok) return;
  try {
    const s = await invoke<Settings>("delete_profile", { id: p.id });
    await applySettings(s);
    editAccount(activeProfile());
  } catch (e) {
    fail(e);
  }
}

// ---------- general tab ----------

const generalForm = () => $<HTMLFormElement>("general-form");
const box = (name: string) => generalForm().elements.namedItem(name) as HTMLInputElement;

async function syncGeneralControls() {
  const g = app.settings.general;
  box("run_in_background").checked = g.run_in_background;
  box("check_updates").checked = g.check_updates;
  box("notify").checked = g.notify;
  box("launch_at_login").checked = await invoke<boolean>("autostart_enabled").catch(() => false);
}

async function onGeneralChange(ev: Event) {
  const name = (ev.target as HTMLInputElement).name;
  try {
    if (name === "launch_at_login") {
      await invoke("set_autostart", { enabled: box(name).checked });
      return;
    }
    const general: General = {
      run_in_background: box("run_in_background").checked,
      check_updates: box("check_updates").checked,
      notify: box("notify").checked,
    };
    app.settings = await invoke<Settings>("save_general", { general });
  } catch (e) {
    fail(e);
    syncGeneralControls();
  }
}

// ---------- appearance tab ----------

function syncAppearanceControls() {
  const check = (name: string, value: string) =>
    dialog()
      .querySelectorAll<HTMLInputElement>(`input[name="${name}"]`)
      .forEach((i) => (i.checked = i.value === value));
  check("theme", appearance.theme);
  check("size", appearance.size);
}

// ---------- account + bucket pickers (remote pane header) ----------

function renderProfilePicker() {
  const sel = $<HTMLSelectElement>("profile");
  const active = activeProfile()?.id;
  sel.innerHTML =
    app.settings.profiles
      .map((p) => `<option value="${esc(p.id)}"${p.id === active ? " selected" : ""}>${esc(p.name)}</option>`)
      .join("") + `<option value="${ADD}">＋ Add account…</option>`;
  sel.hidden = app.settings.profiles.length === 0;
}

/** Adopts settings returned by the backend and reloads the remote pane. */
export async function applySettings(s: Settings) {
  const switched = s.active !== app.settings.active;
  app.settings = s;
  renderProfilePicker();
  if (dialog().open) renderAccountList();
  await loadBuckets(switched);
}

/** Fills the bucket picker and restores this account's last bucket + prefix when possible. */
export async function loadBuckets(resetScroll = false) {
  const sel = $<HTMLSelectElement>("bucket");
  const p = activeProfile();
  let names: string[] = [];
  if (p) {
    try {
      names = await invoke<string[]>("list_buckets");
    } catch (e) {
      fail(e);
    }
  }
  const last = store.get(profileKey("bucket"));
  const wanted = p?.bucket ?? "";
  app.bucket = names.includes(last ?? "") ? last! : names.includes(wanted) ? wanted : (names[0] ?? "");
  sel.innerHTML = names.map((n) => `<option ${n === app.bucket ? "selected" : ""}>${esc(n)}</option>`).join("");
  sel.hidden = !p;
  remote.cwd = app.bucket && app.bucket === last ? (store.get(profileKey("prefix")) ?? "") : "";
  if (resetScroll) remote.list.scrollTop = 0;
  await remote.load();
}

export async function initSettings() {
  app.settings = await invoke<Settings>("get_settings");
  renderProfilePicker();
  if (app.settings.profiles.length) await loadBuckets();
  else {
    remote.render();
    openSettings(); // first run
  }
}

export function setupSettings() {
  form().addEventListener("submit", saveAccount);
  $("acct-test").onclick = testAccount;
  $("acct-delete").onclick = deleteAccount;
  $("acct-add").onclick = () => editAccount(null);
  $<HTMLSelectElement>("acct-provider").addEventListener("change", (ev) =>
    applyPreset((ev.target as HTMLSelectElement).value as ProviderId, true),
  );
  $("acct-items").addEventListener("click", (ev) => {
    const id = (ev.target as HTMLElement).closest<HTMLElement>("[data-id]")?.dataset.id;
    if (id) editAccount(app.settings.profiles.find((p) => p.id === id) ?? null);
  });
  $("settings-close").onclick = () => dialog().close();
  generalForm().addEventListener("change", onGeneralChange);
  dialog()
    .querySelectorAll<HTMLElement>(".check-updates")
    .forEach((b) => (b.onclick = () => checkForUpdates(true)));
  // About tab links open in the default browser.
  dialog().addEventListener("click", (ev) => {
    const url = (ev.target as HTMLElement).closest<HTMLElement>("[data-url]")?.dataset.url;
    if (url) openUrl(url).catch(fail);
  });
  dialog()
    .querySelectorAll<HTMLElement>(".tabs [data-tab]")
    .forEach((b) => (b.onclick = () => showTab(b.dataset.tab!)));
  $("ap-theme").addEventListener("change", (ev) => setTheme((ev.target as HTMLInputElement).value as Theme));
  $("ap-size").addEventListener("change", (ev) => setTextSize((ev.target as HTMLInputElement).value));

  $("btn-settings").onclick = () => openSettings();
  $<HTMLSelectElement>("profile").addEventListener("change", async (ev) => {
    const sel = ev.target as HTMLSelectElement;
    if (sel.value === ADD) {
      renderProfilePicker(); // revert the visible choice
      openSettings("accounts", "");
      editAccount(null);
      return;
    }
    try {
      await applySettings(await invoke<Settings>("set_active_profile", { id: sel.value }));
    } catch (e) {
      fail(e);
    }
  });
  $<HTMLSelectElement>("bucket").addEventListener("change", (ev) => {
    app.bucket = (ev.target as HTMLSelectElement).value;
    remote.go("");
  });
}
