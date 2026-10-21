// Speed test (experimental, ADR-050): moves test data between this computer and a remote, or two
// remotes, the way NoExplorer transfers files (speedtest.rs). Two tabs: Run (pick each end's
// account, bucket and folder explicitly, a size, both directions) and History (the runs kept on
// this device, one line each, expanding to the full numbers and what the cleanup did).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { icon } from "./icons";
import { remotes } from "./panes";
import { badge, guessProvider, hasContainers } from "./providers";
import { app, exp } from "./state";
import type { BucketInfo, Profile } from "./types";
import { esc, fail, fmtSize, store } from "./util";

type End = { account: string | null; bucket: string; prefix: string };
type Measure = { route: string; test: "large" | "small" | "latency"; bytes: number; files: number; secs: number };
type SpeedEvent = {
  id: string;
  phase: string;
  done: number;
  total: number;
  results: Measure[];
  notes: string[];
  finished: boolean;
  error: string | null;
};
type Run = {
  id: string;
  at: number;
  a: End;
  b: End;
  aName: string;
  bName: string;
  size: Size;
  both: boolean;
  results: Measure[];
  notes: string[];
  error?: string;
};

const SIZES = {
  quick: { title: "Quick", sub: "16 MB · 50 small", bigMb: 16, smallCount: 50, smallKb: 128 },
  standard: { title: "Standard", sub: "64 MB · 200 small", bigMb: 64, smallCount: 200, smallKb: 256 },
  thorough: { title: "Thorough", sub: "256 MB · 500 small", bigMb: 256, smallCount: 500, smallKb: 512 },
} as const;
type Size = keyof typeof SIZES;

const HISTORY = "speedTests";
const MAX_RUNS = 30;
let running: Run | null = null;
let size: Size = "standard";
const open = new Set<string>(); // history items expanded
const buckets = new Map<string, BucketInfo[]>();

function history(): Run[] {
  try {
    const runs = JSON.parse(store.get(HISTORY) ?? "[]") as Run[];
    return runs.filter((r) => r && r.id && r.a && r.b); // runs saved by 0.4.32 had another shape
  } catch {
    return [];
  }
}
const saveHistory = (runs: Run[]) => store.set(HISTORY, JSON.stringify(runs.slice(0, MAX_RUNS)));

const profileOf = (id: string | null) => app.settings.profiles.find((p) => p.id === id) ?? null;
const endName = (e: End) => {
  const p = profileOf(e.account);
  if (!p) return "This computer";
  const where = [hasContainers(p) ? e.bucket : "", e.prefix.replace(/\/$/, "")].filter(Boolean).join("/");
  return where ? `${p.name} · ${where}` : p.name;
};

// ---------- the dialog ----------

function dlg() {
  let d = document.getElementById("speedtest") as HTMLDialogElement | null;
  if (d) return d;
  d = document.createElement("dialog");
  d.id = "speedtest";
  d.setAttribute("aria-label", "Speed test");
  const end = (k: "a" | "b", label: string) => `<div class="st-end" data-end="${k}">
      <span class="st-label">${label}</span>
      <div class="st-acct"><span class="st-badge"></span><select aria-label="${label}: account"></select></div>
      <select class="st-bucket" aria-label="${label}: bucket"></select>
      <input class="st-folder" placeholder="Folder (optional)" spellcheck="false" aria-label="${label}: folder" />
    </div>`;
  d.innerHTML = `<div class="st">
    <header class="st-head">
      <b class="st-title">${icon("gauge")} Speed test</b>
      <nav class="st-tabs" role="tablist">
        <button type="button" role="tab" data-tab="run" aria-selected="true">Run</button>
        <button type="button" role="tab" data-tab="history" aria-selected="false">History <span class="st-count"></span></button>
      </nav>
      <button type="button" class="ib" data-close aria-label="Close">${icon("x")}</button>
    </header>
    <section class="st-pane" data-pane="run">
      <div class="st-ends">${end("a", "From")}<button type="button" class="st-swap" title="Swap" aria-label="Swap">⇄</button>${end("b", "To")}</div>
      <p class="st-where"></p>
      <div class="st-sizes" role="radiogroup" aria-label="Size">${Object.entries(SIZES)
        .map(
          ([k, s]) =>
            `<button type="button" role="radio" data-size="${k}"><b>${s.title}</b><small>${s.sub}</small></button>`,
        )
        .join("")}</div>
      <label class="set-row st-both"><span>Both directions<small>Also measure the way back</small></span><input type="checkbox" class="switch" id="st-both" checked /></label>
      <p class="st-msg" id="st-msg"></p>
      <div class="st-go"><button type="button" class="primary" id="st-run">${icon("play")} Run test</button></div>
      <div class="st-live" id="st-live" hidden>
        <div class="st-phase"><span id="st-phase"></span><span id="st-pct"></span></div>
        <div class="st-track"><i id="st-bar"></i></div>
        <div id="st-now"></div>
      </div>
    </section>
    <section class="st-pane" data-pane="history" hidden><ul class="st-list" id="st-list"></ul></section>
  </div>`;
  document.body.append(d);
  d.querySelector("[data-close]")!.addEventListener("click", () => d!.close());
  d.querySelector(".st-tabs")!.addEventListener("click", (ev) => {
    const t = (ev.target as HTMLElement).closest<HTMLElement>("[data-tab]")?.dataset.tab;
    if (t) showTab(t as "run" | "history");
  });
  d.querySelector(".st-sizes")!.addEventListener("click", (ev) => {
    const s = (ev.target as HTMLElement).closest<HTMLElement>("[data-size]")?.dataset.size;
    if (!s) return;
    size = s as Size;
    drawSizes();
  });
  d.querySelector(".st-swap")!.addEventListener("click", () => {
    const a = readEnd("a");
    setEnd("a", readEnd("b"));
    setEnd("b", a);
  });
  for (const k of ["a", "b"] as const) {
    const box = d.querySelector<HTMLElement>(`.st-end[data-end="${k}"]`)!;
    box.querySelector<HTMLSelectElement>(".st-acct select")!.addEventListener("change", (ev) => {
      setEnd(k, defaultEnd((ev.target as HTMLSelectElement).value || null));
    });
    box.querySelector(".st-bucket")!.addEventListener("change", () => drawWhere());
    box.querySelector(".st-folder")!.addEventListener("input", () => drawWhere());
  }
  d.querySelector("#st-run")!.addEventListener("click", () =>
    running ? invoke("cancel", { id: running.id }) : start(),
  );
  d.querySelector("#st-list")!.addEventListener("click", onHistoryClick);
  return d;
}

const q = <T extends HTMLElement>(sel: string) => dlg().querySelector<T>(sel)!;
const endEl = (k: "a" | "b") => q(`.st-end[data-end="${k}"]`);

function showTab(t: "run" | "history") {
  for (const b of dlg().querySelectorAll<HTMLElement>("[data-tab]"))
    b.setAttribute("aria-selected", String(b.dataset.tab === t));
  for (const p of dlg().querySelectorAll<HTMLElement>("[data-pane]")) p.hidden = p.dataset.pane !== t;
  if (t === "history") drawHistory();
}

function drawSizes() {
  for (const b of dlg().querySelectorAll<HTMLElement>("[data-size]"))
    b.setAttribute("aria-checked", String(b.dataset.size === size));
}

/** Where an account is tested by default: the folder an open view shows, else its default bucket. */
function defaultEnd(account: string | null): End {
  if (!account) return { account: null, bucket: "", prefix: "" };
  const v = remotes.find((r) => r.account === account && r.bucket);
  if (v) return { account, bucket: v.bucket, prefix: v.cwd };
  return { account, bucket: profileOf(account)?.bucket ?? "", prefix: "" };
}

async function bucketsOf(p: Profile) {
  if (!buckets.has(p.id)) buckets.set(p.id, await invoke<BucketInfo[]>("list_buckets", { account: p.id }));
  return buckets.get(p.id)!;
}

/** Shows `e` in an end's controls; buckets load from the account when it has several. */
async function setEnd(k: "a" | "b", e: End) {
  const box = endEl(k);
  const acct = box.querySelector<HTMLSelectElement>(".st-acct select")!;
  const p = profileOf(e.account);
  acct.innerHTML =
    `<option value="">This computer</option>` +
    app.settings.profiles.map((x) => `<option value="${esc(x.id)}">${esc(x.name)}</option>`).join("");
  acct.value = p?.id ?? "";
  box.querySelector(".st-badge")!.innerHTML = p ? badge(guessProvider(p), true) : icon("desktop");
  const sel = box.querySelector<HTMLSelectElement>(".st-bucket")!;
  const folder = box.querySelector<HTMLInputElement>(".st-folder")!;
  folder.value = e.prefix;
  folder.hidden = !p;
  sel.hidden = !p || !hasContainers(p);
  sel.innerHTML = e.bucket ? `<option>${esc(e.bucket)}</option>` : "";
  drawWhere();
  if (!p || !hasContainers(p)) return;
  try {
    const list = await bucketsOf(p);
    if (acct.value !== p.id) return; // changed meanwhile
    const pick = e.bucket || list[0]?.name || "";
    sel.innerHTML = list.map((b) => `<option value="${esc(b.name)}">${esc(b.name)}</option>`).join("");
    if (pick && !list.some((b) => b.name === pick)) sel.add(new Option(pick, pick));
    sel.value = pick;
    if (!pick) q("#st-msg").textContent = `✗ ${p.name}: no bucket visible with this key`;
  } catch (err) {
    q("#st-msg").textContent = `✗ ${p.name}: ${err}`;
  }
  drawWhere();
}

function readEnd(k: "a" | "b"): End {
  const box = endEl(k);
  const account = box.querySelector<HTMLSelectElement>(".st-acct select")!.value || null;
  if (!account) return { account: null, bucket: "", prefix: "" };
  const folder = box.querySelector<HTMLInputElement>(".st-folder")!.value.trim().replace(/^\/+/, "");
  return {
    account,
    bucket: box.querySelector<HTMLSelectElement>(".st-bucket")!.value,
    prefix: folder && !folder.endsWith("/") ? `${folder}/` : folder,
  };
}

/** Says exactly where test data goes: no surprises in a bucket. */
function drawWhere() {
  const places = [readEnd("a"), readEnd("b")].filter((e) => e.account);
  q(".st-where").innerHTML = places.length
    ? `Test data goes in ${places.map((e) => `<code>${esc(endName(e))}/.noexplorer-speedtest-…</code>`).join(" and ")}, and is deleted afterwards.`
    : "Pick a remote on at least one side.";
}

/** Opens the dialog; `to` = a remote view to preselect as the other end. */
export function openSpeedTest(to?: { account: string; bucket: string; cwd: string }) {
  const d = dlg();
  if (!running) {
    q("#st-msg").textContent = "";
    setEnd("a", defaultEnd(null));
    setEnd(
      "b",
      to
        ? { account: to.account, bucket: to.bucket, prefix: to.cwd }
        : defaultEnd(app.settings.profiles[0]?.id ?? null),
    );
  }
  drawSizes();
  drawCount();
  showTab("run");
  if (!d.open) d.showModal();
}

// ---------- running ----------

async function start() {
  const a = readEnd("a");
  const b = readEnd("b");
  const msg = q("#st-msg");
  msg.textContent = "";
  if (!a.account && !b.account) return void (msg.textContent = "Pick a remote on at least one side.");
  if (JSON.stringify(a) === JSON.stringify(b)) return void (msg.textContent = "Pick two different places.");
  for (const e of [a, b]) {
    const p = profileOf(e.account);
    if (p && hasContainers(p) && !e.bucket) return void (msg.textContent = `Pick a bucket for ${p.name}.`);
    // Single-container accounts (WebDAV, drives) use their one container.
    if (p && !hasContainers(p)) e.bucket = (await bucketsOf(p).catch(() => []))[0]?.name ?? e.bucket;
  }
  const run: Run = {
    id: `st-${Date.now()}`,
    at: Date.now(),
    a,
    b,
    aName: endName(a),
    bName: endName(b),
    size,
    both: q<HTMLInputElement>("#st-both").checked,
    results: [],
    notes: [],
  };
  const { bigMb, smallCount, smallKb } = SIZES[size];
  running = run;
  setRunning(true);
  drawLive(null);
  try {
    await invoke("speed_test", {
      id: run.id,
      a: { name: run.aName, ...a },
      b: { name: run.bName, ...b },
      plan: { bigMb, smallCount, smallKb, both: run.both },
    });
  } catch (e) {
    running = null;
    setRunning(false);
    q("#st-live").hidden = true;
    msg.textContent = `✗ ${e}`;
  }
}

function setRunning(on: boolean) {
  q("#st-run").innerHTML = on ? `${icon("x")} Cancel` : `${icon("play")} Run test`;
  for (const el of dlg().querySelectorAll<HTMLInputElement>(
    ".st-end select, .st-end input, .st-swap, [data-size], #st-both",
  ))
    el.disabled = on;
}

function drawLive(e: SpeedEvent | null) {
  q("#st-live").hidden = false;
  q("#st-phase").textContent = e?.phase || "Starting…";
  const pct = e?.total ? Math.min(100, (e.done / e.total) * 100) : 0;
  q("#st-pct").textContent = e?.total ? `${Math.round(pct)}%` : "";
  q<HTMLElement>("#st-bar").style.width = `${pct}%`;
  q("#st-now").innerHTML = e?.results.length ? table(e.results) : "";
}

// ---------- numbers ----------

const speed = (m?: Measure) => (m && m.secs > 0 ? m.bytes / m.secs : 0);
const fmtSpeed = (bps: number) => `${fmtSize(bps)}/s`;
const secs = (s: number) => (s < 10 ? `${s.toFixed(1)} s` : `${Math.round(s)} s`);

function table(results: Measure[]) {
  const routes = [...new Set(results.filter((m) => m.test !== "latency").map((m) => m.route))];
  const rows = routes.map((r) => {
    const large = results.find((m) => m.route === r && m.test === "large");
    const small = results.find((m) => m.route === r && m.test === "small");
    return `<tr><th scope="row">${esc(r)}</th>
      <td>${large ? `<b>${fmtSpeed(speed(large))}</b><small>${fmtSize(large.bytes)} in ${secs(large.secs)}</small>` : "…"}</td>
      <td>${small ? `<b>${(small.files / small.secs).toFixed(1)} files/s</b><small>${small.files} files · ${fmtSpeed(speed(small))}</small>` : "…"}</td></tr>`;
  });
  const lat = results.filter((m) => m.test === "latency");
  return `<table class="st-table"><thead><tr><th>Route</th><th>One large file</th><th>Many small files</th></tr></thead><tbody>${rows.join("")}</tbody></table>${
    lat.length
      ? `<p class="st-lat">Response time ${lat.map((m) => `<span class="chip">${esc(m.route)} <b>${Math.round(m.secs * 1000)} ms</b></span>`).join("")}</p>`
      : ""
  }`;
}

// ---------- history ----------

function drawCount() {
  const n = history().length;
  q(".st-count").textContent = n ? String(n) : "";
}

const failedCleanup = (r: Run) => r.notes.some((n) => n.startsWith("Couldn't"));

function drawHistory() {
  const runs = history();
  const list = q("#st-list");
  if (!runs.length) {
    list.innerHTML = `<li class="st-empty">No runs yet. Each run is kept here, on this computer, so you can compare services.</li>`;
    return;
  }
  const best = Math.max(0, ...runs.flatMap((r) => r.results.filter((m) => m.test === "large").map(speed)));
  const when = (t: number) => new Date(t).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  const chip = (arrow: string, m?: Measure) => (m ? `<span class="chip">${arrow} ${fmtSpeed(speed(m))}</span>` : "");
  list.innerHTML =
    runs
      .map((r) => {
        const there = r.results.find((m) => m.test === "large" && m.route === `${r.aName} → ${r.bName}`);
        const back = r.results.find((m) => m.test === "large" && m.route === `${r.bName} → ${r.aName}`);
        const lat = r.results.filter((m) => m.test === "latency");
        const top = [there, back].some((m) => m && best > 0 && speed(m) >= best);
        const warn = !!r.error || failedCleanup(r);
        const expanded = open.has(r.id);
        const detail = expanded
          ? `<div class="st-detail">${r.results.length ? table(r.results) : ""}${
              r.error ? `<p class="st-note warn">${esc(r.error === "cancelled" ? "Cancelled." : r.error)}</p>` : ""
            }${r.notes.map((n) => `<p class="st-note${n.startsWith("Couldn't") ? " warn" : ""}">${esc(n)}</p>`).join("")}
            <p class="st-meta">${esc(SIZES[r.size] ? `${SIZES[r.size].title}: ${SIZES[r.size].sub}` : "")}${r.both ? " · both directions" : ""} · ${failedCleanup(r) ? "some test data is left (see above)" : "test data removed"}</p>
            <div class="st-actions"><button type="button" data-again>${icon("refresh")} Run again</button><button type="button" data-remove>${icon("trash")} Remove from history</button></div></div>`
          : "";
        return `<li class="st-item${expanded ? " open" : ""}" data-id="${esc(r.id)}">
        <button type="button" class="st-sum" aria-expanded="${expanded}">
          <span class="st-sum-main"><b>${esc(r.aName)} ${r.both ? "⇄" : "→"} ${esc(r.bName)}</b><small>${esc(when(r.at))} · ${SIZES[r.size]?.title ?? ""}</small></span>
          <span class="st-chips">${chip("→", there)}${chip("←", back)}${lat.length ? `<span class="chip">${Math.round(Math.max(...lat.map((m) => m.secs)) * 1000)} ms</span>` : ""}${top ? `<span class="chip best">★ fastest</span>` : ""}</span>
          <span class="st-flag ${warn ? "warn" : "ok"}" title="${warn ? "Something went wrong: see details" : "Test data removed"}">${warn ? "!" : "✓"}</span>
          ${icon(expanded ? "chevDown" : "chevRight")}
        </button>${detail}
      </li>`;
      })
      .join("") + `<li class="st-foot"><button type="button" class="ep-link" data-clear>Clear history</button></li>`;
}

function onHistoryClick(ev: Event) {
  const t = ev.target as HTMLElement;
  if (t.closest("[data-clear]")) {
    saveHistory([]);
    drawCount();
    return drawHistory();
  }
  const item = t.closest<HTMLElement>(".st-item");
  if (!item) return;
  const id = item.dataset.id!;
  const run = history().find((r) => r.id === id);
  if (t.closest("[data-remove]")) {
    saveHistory(history().filter((r) => r.id !== id));
    drawCount();
    return drawHistory();
  }
  if (t.closest("[data-again]") && run && !running) {
    setEnd("a", run.a);
    setEnd("b", run.b);
    size = run.size in SIZES ? run.size : "standard";
    drawSizes();
    q<HTMLInputElement>("#st-both").checked = run.both;
    showTab("run");
    return void start();
  }
  if (t.closest(".st-sum")) {
    if (open.has(id)) open.delete(id);
    else open.add(id);
    drawHistory();
  }
}

export function setupSpeedTest() {
  listen<SpeedEvent>("speedtest", (ev) => {
    const e = ev.payload;
    if (!running || e.id !== running.id) return;
    drawLive(e);
    if (!e.finished) return;
    const run: Run = { ...running, results: e.results, notes: e.notes, error: e.error ?? undefined };
    running = null;
    setRunning(false);
    q("#st-phase").textContent =
      e.error === "cancelled" ? "Cancelled; test data removed." : e.error ? `✗ ${e.error}` : "Done · saved to History";
    q("#st-pct").textContent = "";
    if (e.results.length || (e.error && e.error !== "cancelled")) {
      saveHistory([run, ...history()]);
      open.add(run.id);
      drawCount();
    }
  }).catch(fail);
}

/** For ⌘K and menus: is the experiment on and is there a remote to test? */
export const canSpeedTest = () => exp("speed_test") && app.settings.profiles.length > 0;
