// Settings dialog (Accounts / General / Appearance / About tabs), the account picker and the
// bucket picker in the remote pane header.

import { favoritesMenuItems, openFavorite } from "./nav";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { appearance, setTextSize, setTheme, type Theme } from "./appearance";
import { addView, MAX_VIEWS, remote, remotes, savedViews, saveViews, switchTo } from "./panes";
import { type Pane, paneHooks } from "./pane";
import {
  describeAccount,
  EXPERIMENTAL_PROVIDERS,
  guessProvider,
  hasContainers,
  PRESETS,
  type ProviderId,
} from "./providers";
import { activeProfile, app, exp } from "./state";
import { openMenu, type MenuItem } from "./popover";
import type { BucketInfo, General, Profile, Settings } from "./types";
import { renderExperimental } from "./experimental";
import { requestSizes, stopSizes } from "./sizes";
import { checkForUpdates } from "./updater";
import { $, esc, fail, plural } from "./util";

const dialog = () => $<HTMLDialogElement>("settings");
const form = () => $<HTMLFormElement>("acct-form");
const FIELDS = ["name", "endpoint", "region", "access_key", "secret_key", "bucket"] as const;

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
  if (tab === "experimental") renderExperimental();
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
  // The backend never sends a saved secret back: leave the field empty to keep it.
  const secret = f.elements.namedItem("secret_key") as HTMLInputElement;
  secret.required = !p;
  secret.placeholder = p ? (p.keychain ? "Saved in your system keychain" : "Saved") + " · type to replace" : "";
  const provider = p ? guessProvider(p) : "custom";
  syncProviderOptions();
  if (EXPERIMENTAL_PROVIDERS.includes(provider) && !$("acct-provider").querySelector(`option[value="${provider}"]`))
    $<HTMLSelectElement>("acct-provider").add(new Option(PRESETS[provider].label, provider));
  $<HTMLSelectElement>("acct-provider").value = provider;
  applyPreset(provider, false);
  // Nextcloud can sign in through the browser instead: no password to type.
  secret.required = !p && provider !== "nextcloud";
  $<HTMLButtonElement>("acct-delete").hidden = !p;
  input("cert_pin").value = p?.cert_pin ?? "";
  renderCert();
  setMsg("");
  renderAccountList();
  (f.elements.namedItem(p ? "name" : "endpoint") as HTMLInputElement).focus();
}

const input = (name: string) => form().elements.namedItem(name) as HTMLInputElement;

/** Updates hints/placeholders for a provider. `fill` = the user just picked it: also reset
 * fields that belong to another provider and set the provider's defaults. */
function applyPreset(id: ProviderId, fill: boolean) {
  const pr = PRESETS[id];
  const s3 = pr.kind === "s3";
  $("acct-hint").textContent = pr.hint;
  const endpoint = input("endpoint");
  endpoint.placeholder = pr.endpointPlaceholder;
  endpoint.required = pr.endpointRequired;
  input("region").placeholder = pr.regionPlaceholder;
  form()
    .querySelectorAll<HTMLElement>(".s3-only")
    .forEach((el) => (el.hidden = !s3));
  const box = pr.kind === "box";
  $("acct-signin").hidden = pr.kind !== "nextcloud" && !box;
  $("acct-nc-login").textContent = box ? "Sign in with Box…" : "Sign in with Nextcloud…";
  $("acct-signin-hint").textContent = box
    ? "Opens your browser to approve your Box app. Fill in the Client ID and secret below first."
    : "Opens your browser. Or enter a user name and an app password below.";
  $("acct-endpoint-row").hidden = box;
  endpoint.required = pr.endpointRequired && !box;
  const azure = pr.kind === "azure";
  const sftp = pr.kind === "sftp";
  $("lbl-endpoint").textContent = s3
    ? "Endpoint"
    : pr.kind === "nextcloud" || sftp
      ? "Server address"
      : azure
        ? "Blob endpoint"
        : "WebDAV URL";
  $("lbl-access").textContent = s3
    ? "Access key"
    : box
      ? "Client ID"
      : azure
        ? "Storage account name"
        : sftp
          ? "User name (optional)"
          : "User name";
  $("lbl-secret").textContent = s3
    ? "Secret key"
    : box
      ? "Client secret"
      : azure
        ? "Account key"
        : pr.kind === "nextcloud"
          ? "App password"
          : "Password";
  input("access_key").required = pr.kind !== "nextcloud" && !sftp;
  // SFTP logs in with SSH keys: no secret to type.
  input("secret_key").closest("label")!.hidden = sftp;
  if (sftp) input("secret_key").required = false;
  if (!fill) return;
  if (guessProvider({ endpoint: endpoint.value }) !== id || !s3) endpoint.value = "";
  input("region").value = pr.region;
  input("path_style").checked = pr.pathStyle;
  (pr.endpointRequired ? endpoint : input("access_key")).focus();
}

function readForm(): Profile {
  const f = new FormData(form());
  const v = (k: string) => String(f.get(k) ?? "").trim();
  return {
    id: editing,
    kind: PRESETS[$<HTMLSelectElement>("acct-provider").value as ProviderId].kind,
    name: v("name"),
    endpoint: v("endpoint"),
    region: v("region"),
    access_key: v("access_key"),
    secret_key: v("secret_key"),
    bucket: v("bucket"),
    path_style: f.get("path_style") === "on",
    cert_pin: v("cert_pin"),
  };
}

// ---------- experimental providers ----------

/** Adds or removes GCS / Azure / SFTP in the provider picker (Safari can't hide <option>s). */
function syncProviderOptions() {
  const sel = $<HTMLSelectElement>("acct-provider");
  const on = exp("providers");
  for (const id of EXPERIMENTAL_PROVIDERS) {
    const opt = sel.querySelector<HTMLOptionElement>(`option[value="${id}"]`);
    // Keep an option the open account already uses, even with the experiment off.
    if (!on && opt && sel.value !== id) opt.remove();
    if (on && !opt) sel.add(new Option(`${PRESETS[id].label} (experimental)`, id));
  }
}

// ---------- self-signed HTTPS (experimental) ----------

/** The "Certificate" row of the account form: shown for https:// servers when the experiment is
 * on (or when the account already trusts a certificate, so it can be removed). */
function renderCert() {
  const pin = input("cert_pin").value;
  const kind = PRESETS[$<HTMLSelectElement>("acct-provider").value as ProviderId].kind;
  const https = /^https:\/\//i.test(input("endpoint").value.trim());
  $("acct-cert").hidden = !pin && !(exp("self_signed") && https && kind !== "box");
  $("acct-cert-text").textContent = pin
    ? `Trusts the certificate ${pin.slice(0, 11)}…${pin.slice(-5)}`
    : "Self-signed certificate? Trust it after checking its fingerprint.";
  $("acct-cert-text").title = pin;
  $("acct-cert-btn").textContent = pin ? "Stop trusting" : "Trust certificate…";
}

async function onCertButton() {
  if (input("cert_pin").value) {
    input("cert_pin").value = "";
    return renderCert();
  }
  const endpoint = input("endpoint").value.trim();
  try {
    const fp = await invoke<string>("cert_fingerprint", { endpoint });
    const host = endpoint.replace(/^https:\/\//i, "").split("/")[0];
    const ok = await ask(
      `Trust the certificate of ${host}?\n\nSHA-256 fingerprint:\n${fp}\n\nCompare it with the fingerprint your server shows (for example in its admin page). Only trust it if they match.`,
      { title: "Trust certificate", kind: "warning", okLabel: "Trust", cancelLabel: "Cancel" },
    );
    if (ok) input("cert_pin").value = fp;
    renderCert();
  } catch (e) {
    fail(e);
  }
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
    const what = hasContainers(readForm())
      ? `${plural(b.length, readForm().kind === "azure" ? "container" : "bucket")}: ${b.slice(0, 8).join(", ")}${b.length > 8 ? "…" : ""}`
      : "files are reachable";
    setMsg(`✓ Connected · ${what}`, "ok");
    return true;
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
    return false;
  }
}

/** Save = test first, then persist and switch the remote pane to this account. */
async function saveAccount(ev: Event) {
  ev.preventDefault();
  // Box accounts are created by signing in (that's where the tokens come from).
  if (readForm().kind === "box") return boxSignIn();
  if (!(await testAccount())) return;
  try {
    const s = await invoke<Settings>("save_profile", { profile: readForm() });
    await applySettings(s, s.active);
    dialog().close();
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
  }
}

/** Box with the user's own OAuth app: the browser approves, the backend saves the account and
 * its tokens (keychain). */
async function boxSignIn() {
  const clientId = input("access_key").value.trim();
  if (!clientId) return (setMsg("Enter your Box app's Client ID first.", "err"), input("access_key").focus());
  const btn = $<HTMLButtonElement>("acct-nc-login");
  btn.disabled = true;
  $("acct-nc-cancel").hidden = false;
  setMsg("Approve the app in the browser window that just opened…");
  try {
    const s = await invoke<Settings>("box_login", {
      clientId,
      clientSecret: input("secret_key").value,
      id: editing || null,
      name: input("name").value,
    });
    await applySettings(s, s.active);
    dialog().close();
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
  } finally {
    btn.disabled = false;
    $("acct-nc-cancel").hidden = true;
  }
}

/** Nextcloud Login Flow v2: the browser approves, the backend saves the account (and its app
 * password, straight into the keychain). */
async function nextcloudSignIn() {
  const server = input("endpoint").value.trim();
  if (!server) return (setMsg("Enter your Nextcloud address first.", "err"), input("endpoint").focus());
  const btn = $<HTMLButtonElement>("acct-nc-login");
  btn.disabled = true;
  $("acct-nc-cancel").hidden = false;
  setMsg("Approve NoExplorer in the browser window that just opened…");
  try {
    const s = await invoke<Settings>("nextcloud_login", { server, id: editing || null, name: input("name").value });
    await applySettings(s, s.active);
    dialog().close();
  } catch (e) {
    setMsg(`✗ ${e}`, "err");
  } finally {
    btn.disabled = false;
    $("acct-nc-cancel").hidden = true;
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
  box("auto_sizes").checked = g.auto_sizes;
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
      ...app.settings.general,
      run_in_background: box("run_in_background").checked,
      check_updates: box("check_updates").checked,
      auto_sizes: box("auto_sizes").checked,
    };
    app.settings = await invoke<Settings>("save_general", { general });
    if (name === "auto_sizes") general.auto_sizes ? requestSizes() : stopSizes();
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

// ---------- account + bucket pickers (each remote view's header) ----------

/** Updates a remote view's two chips. */
export function renderPickers(v: Pane) {
  const p = v.profile();
  v.q(".profile-label").textContent = p ? p.name : "Add account";
  v.q(".profile-sub").textContent = p && describeAccount(p) !== p.name ? describeAccount(p) : "";
  v.q(".bucket-label").textContent = v.bucket || (p ? "No bucket" : "—");
  // WebDAV/Nextcloud/Box/SFTP have a single container: no bucket picker to show.
  const buckets = !!p && hasContainers(p);
  const chip = v.q(".bucket-chip");
  chip.hidden = !buckets;
  chip.previousElementSibling!.toggleAttribute("hidden", !buckets); // the › between the chips
}

/** Shows account `id` in view `v`. It also becomes the default account (tray ✓, new views). */
export async function setViewAccount(v: Pane, id: string) {
  v.account = id;
  v.bucket = "";
  v.cwd = "";
  // Saving the default (config + tray menu) doesn't need to hold up the listing.
  invoke<Settings>("set_active_profile", { id }).then((s) => (app.settings = s), fail);
  await loadBuckets(v, true);
}

/** Shows `bucket/prefix` of `account`: in a view already on it, else in the focused view. */
export async function openRemote(account: string, bucket: string, prefix: string) {
  const v = remotes.find((x) => x.account === account && x.bucket === bucket) ?? remote;
  if (v.account === account && v.bucket === bucket) return v.go(prefix);
  v.account = account;
  v.bucket = bucket;
  v.cwd = prefix;
  await loadBuckets(v, true); // keeps the bucket and folder when they exist
}

/** Palette / tray: show `id` in the focused remote view. */
export const switchProfile = (id: string) => setViewAccount(remote, id);

export function selectBucket(v: Pane, name: string) {
  v.bucket = name;
  renderPickers(v);
  v.go("");
}

export function addAccount() {
  openSettings("accounts", "");
  editAccount(null);
}

export function openProfileMenu(v: Pane, anchor = v.q(".profile-chip")) {
  const items: MenuItem[] = [{ header: "Accounts" }];
  for (const p of app.settings.profiles) {
    items.push({
      label: p.name,
      sub: describeAccount(p),
      icon: "cloud",
      iconClass: "acct-ico",
      checked: p.id === v.account,
      run: () => p.id !== v.account && setViewAccount(v, p.id),
    });
  }
  const favs = favoritesMenuItems("remote", (f) => openFavorite(f, v));
  if (favs.length) items.push({ header: "Favorites" }, ...favs);
  if (app.settings.profiles.length) items.push({ sep: true });
  items.push({ label: "Add account…", icon: "plus", run: addAccount });
  items.push({ label: "Manage accounts", icon: "sliders", badge: "⌘,", run: () => openSettings("accounts") });
  openMenu(anchor, items, { width: 300 });
}

export function openBucketMenu(v: Pane, anchor = v.q(".bucket-chip")) {
  const items: MenuItem[] = v.buckets.map((b) => ({
    label: b.name,
    icon: "bucket",
    iconClass: "bucket-ico",
    badge: b.region ?? undefined,
    checked: b.name === v.bucket,
    run: () => b.name !== v.bucket && selectBucket(v, b.name),
  }));
  if (!items.length) items.push({ header: "No buckets visible with this key" });
  openMenu(anchor, items, { width: 300, search: v.buckets.length > 6 ? "Find bucket…" : undefined });
}

/** Wires a new remote view's chips (called through `paneHooks.created`). */
export function bindPickers(v: Pane) {
  v.q(".profile-chip").onclick = () => (app.settings.profiles.length ? openProfileMenu(v) : addAccount());
  v.q(".bucket-chip").onclick = () => openBucketMenu(v);
}

/**
 * Adopts settings returned by the backend and reloads the views. `show` = an account that was
 * just added or signed in: the focused view switches to it.
 */
export async function applySettings(s: Settings, show?: string) {
  app.settings = s;
  knownBuckets.clear(); // an account's endpoint or key may have changed
  if (show) {
    remote.account = show;
    remote.bucket = "";
    remote.cwd = "";
  }
  // Views on a removed account fall back to the default account.
  for (const v of remotes) if (!v.profile()) v.account = s.active;
  if (dialog().open) renderAccountList();
  await Promise.all(remotes.map((v) => loadBuckets(v)));
}

/** Each account's containers, from the last time they were listed. Switching back to an account
 * lists its folder straight away and refreshes this in the background: one round trip instead of
 * two (for Nextcloud/WebDAV/Box, listing containers is only a connection check). */
const knownBuckets = new Map<string, BucketInfo[]>();

/** Picks the view's bucket from `buckets`: keeps it, else the account's default, else the first. */
function useBuckets(v: Pane, buckets: BucketInfo[]) {
  const names = buckets.map((b) => b.name);
  const keep = names.includes(v.bucket);
  v.buckets = buckets;
  v.bucket = keep ? v.bucket : names.includes(v.profile()?.bucket ?? "") ? v.profile()!.bucket : (names[0] ?? "");
  if (!keep) v.cwd = "";
  renderPickers(v);
  return keep;
}

/** Fills a view's bucket picker, keeps its bucket (or the account's default), and lists it. */
export async function loadBuckets(v: Pane, resetScroll = false) {
  const t = ++v.token;
  const p = v.profile();
  renderPickers(v); // the chip shows the new account right away
  if (resetScroll) v.list.scrollTop = 0;
  if (!p) return void (useBuckets(v, []), v.load());
  const fetch = invoke<BucketInfo[]>("list_buckets", { account: v.account });
  const cached = knownBuckets.get(v.account);
  if (cached) {
    useBuckets(v, cached);
    const listing = v.load();
    fetch.then(
      (b) => {
        knownBuckets.set(p.id, b);
        // The bucket went away (or this view moved on): pick again and re-list.
        if (t === v.token && !useBuckets(v, b)) v.load();
      },
      () => {}, // the listing reports connection problems
    );
    return listing;
  }
  if (!v.entries.length || resetScroll) v.showConnecting();
  let buckets: BucketInfo[];
  try {
    buckets = await fetch;
  } catch (e) {
    if (t === v.token) v.showError(e);
    return;
  }
  knownBuckets.set(p.id, buckets);
  if (t !== v.token) return; // switched to another account meanwhile
  useBuckets(v, buckets);
  await v.load();
}

/** Boot: settings, then the saved views (or one view on the default account). */
export async function initSettings() {
  app.settings = await invoke<Settings>("get_settings");
  window.dispatchEvent(new Event("settings-loaded"));
  const saved = savedViews().filter((v) => app.settings.profiles.some((p) => p.id === v.account));
  const views = saved.length ? saved : [{ account: app.settings.active, bucket: "", prefix: "" }];
  for (const v of views) addView(v);
  if (app.settings.profiles.length) await Promise.all(remotes.map((v) => loadBuckets(v)));
  else {
    remotes.forEach((v) => (renderPickers(v), v.render()));
    openSettings(); // first run
  }
}

/** ⌘\ / the split button: a new view below `after`, on the same account. */
export async function splitView(after: Pane = remote) {
  if (remotes.length >= MAX_VIEWS) return;
  const v = addView({ account: after.account || app.settings.active, bucket: after.bucket, prefix: "" }, after);
  switchTo(v);
  await loadBuckets(v);
  saveViews();
}

export function setupSettings() {
  // "Try again" in a view's error: reconnect from scratch.
  paneHooks.retry = (v) => {
    knownBuckets.delete(v.account);
    loadBuckets(v);
  };
  // Tray menu → open the window on that account (tray.rs).
  listen<string>("tray-open-account", async (ev) => {
    if (dialog().open) dialog().close();
    if (remote.account !== ev.payload) await switchProfile(ev.payload);
    switchTo(remote);
  });
  listen("tray-add-account", addAccount);
  form().addEventListener("submit", saveAccount);
  $("acct-test").onclick = testAccount;
  $("acct-nc-login").onclick = () => (readForm().kind === "box" ? boxSignIn() : nextcloudSignIn());
  $("acct-nc-cancel").onclick = () => invoke("cancel_login");
  $("acct-delete").onclick = deleteAccount;
  $("acct-add").onclick = () => editAccount(null);
  // Self-signed HTTPS (experimental): the certificate row follows the address and provider.
  input("endpoint").addEventListener("input", renderCert);
  $("acct-provider").addEventListener("change", renderCert);
  $("acct-cert-btn").onclick = onCertButton;
  window.addEventListener("experimental", syncProviderOptions);
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
}
