// Speed test (experimental, ADR-050): moves test data between this computer and a remote, or two
// remotes, the way NoExplorer transfers files (speedtest.rs), and keeps the results on this device
// so services can be compared side by side. One large file shows throughput; many small files
// show the per-file cost; response time is a listing's round trip.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { remotes } from "./panes";
import { app, exp } from "./state";
import type { BucketInfo } from "./types";
import { esc, fail, fmtSize, store } from "./util";

type Node = { name: string; account: string | null; bucket: string; prefix: string };
type Measure = { route: string; test: "large" | "small" | "latency"; bytes: number; files: number; secs: number };
type SpeedEvent = {
  id: string;
  phase: string;
  done: number;
  total: number;
  results: Measure[];
  finished: boolean;
  error: string | null;
};
type Run = { at: number; size: string; results: Measure[] };

const PRESETS = {
  quick: { label: "Quick · 16 MB + 50 × 128 KB", bigMb: 16, smallCount: 50, smallKb: 128 },
  standard: { label: "Standard · 64 MB + 200 × 256 KB", bigMb: 64, smallCount: 200, smallKb: 256 },
  thorough: { label: "Thorough · 256 MB + 500 × 512 KB", bigMb: 256, smallCount: 500, smallKb: 512 },
} as const;
type Preset = keyof typeof PRESETS;

const HISTORY = "speedTests";
let running: string | null = null;

const history = (): Run[] => {
  try {
    return JSON.parse(store.get(HISTORY) ?? "[]");
  } catch {
    return [];
  }
};

/** Places to test: this computer, the open remote views (their folder), then other accounts. */
function places(): { key: string; label: string; node: () => Promise<Node> }[] {
  const out: { key: string; label: string; node: () => Promise<Node> }[] = [
    {
      key: "local",
      label: "This computer",
      node: async () => ({ name: "This computer", account: null, bucket: "", prefix: "" }),
    },
  ];
  const shown = new Set<string>();
  for (const v of remotes) {
    const p = v.profile();
    if (!p || !v.bucket) continue;
    const name = `${p.name} · ${v.bucket}`;
    shown.add(p.id);
    out.push({
      key: `view:${p.id}:${v.bucket}:${v.cwd}`,
      label: v.cwd ? `${name}/${v.cwd.replace(/\/$/, "")}` : name,
      node: async () => ({ name, account: p.id, bucket: v.bucket, prefix: v.cwd }),
    });
  }
  for (const p of app.settings.profiles) {
    if (shown.has(p.id)) continue;
    out.push({
      key: `acct:${p.id}`,
      label: p.name,
      node: async () => {
        // An account no view shows: its default bucket, else the first one it lists.
        const bucket = p.bucket || (await invoke<BucketInfo[]>("list_buckets", { account: p.id }))[0]?.name || "";
        if (!bucket) throw new Error(`${p.name} has no bucket to test in`);
        return { name: `${p.name} · ${bucket}`, account: p.id, bucket, prefix: "" };
      },
    });
  }
  return out;
}

function dlg() {
  let d = document.getElementById("speedtest") as HTMLDialogElement | null;
  if (d) return d;
  d = document.createElement("dialog");
  d.id = "speedtest";
  d.setAttribute("aria-label", "Speed test");
  d.innerHTML = `<form method="dialog" class="st-form">
    <header><b>Speed test</b><span class="spacer"></span><button type="button" class="ib" id="st-close" aria-label="Close">✕</button></header>
    <p class="st-intro">Moves test data between two places the way NoExplorer moves your files, then deletes it. Remote test data goes in a temporary <code>.noexplorer-speedtest-…</code> folder.</p>
    <div class="st-route">
      <label>From <select id="st-a"></select></label>
      <button type="button" id="st-swap" title="Swap" aria-label="Swap">⇄</button>
      <label>To <select id="st-b"></select></label>
    </div>
    <div class="st-opts">
      <label>Size <select id="st-size">${Object.entries(PRESETS)
        .map(([k, p]) => `<option value="${k}"${k === "standard" ? " selected" : ""}>${esc(p.label)}</option>`)
        .join("")}</select></label>
      <label class="check"><input type="checkbox" id="st-both" checked /> Both directions</label>
      <span class="spacer"></span>
      <button type="submit" class="primary" id="st-run">Run</button>
    </div>
    <div id="st-progress" class="st-progress" hidden><span id="st-phase"></span><div class="st-track"><i id="st-bar"></i></div></div>
    <p id="st-msg" class="st-msg"></p>
    <div id="st-results" class="st-results"></div>
  </form>`;
  document.body.append(d);
  const q = <T extends HTMLElement>(id: string) => d!.querySelector<T>(`#${id}`)!;
  q("st-close").onclick = () => d!.close();
  q("st-swap").onclick = () => {
    const a = q<HTMLSelectElement>("st-a");
    const b = q<HTMLSelectElement>("st-b");
    [a.value, b.value] = [b.value, a.value];
  };
  d.querySelector("form")!.addEventListener("submit", (ev) => {
    ev.preventDefault();
    if (running) invoke("cancel", { id: running });
    else start();
  });
  d.addEventListener("click", (ev) => {
    if ((ev.target as HTMLElement).closest("[data-clear]")) {
      store.set(HISTORY, "[]");
      drawResults(null);
    }
  });
  return d;
}

const el = <T extends HTMLElement>(id: string) => dlg().querySelector<T>(`#${id}`)!;

/** Opens the dialog; `to` = a remote view to preselect as the other end. */
export function openSpeedTest(to?: { account: string; bucket: string; cwd: string }) {
  const d = dlg();
  const list = places();
  const fill = (sel: HTMLSelectElement, value: string) => {
    const keep = sel.value;
    sel.innerHTML = list.map((p) => `<option value="${esc(p.key)}">${esc(p.label)}</option>`).join("");
    sel.value = list.some((p) => p.key === keep) ? keep : value;
  };
  const view = to && list.find((p) => p.key === `view:${to.account}:${to.bucket}:${to.cwd}`);
  fill(el("st-a"), "local");
  fill(el("st-b"), view?.key ?? list[1]?.key ?? "local");
  if (view) {
    el<HTMLSelectElement>("st-a").value = "local";
    el<HTMLSelectElement>("st-b").value = view.key;
  }
  drawResults(null);
  if (!d.open) d.showModal();
}

async function start() {
  const list = places();
  const pa = list.find((p) => p.key === el<HTMLSelectElement>("st-a").value);
  const pb = list.find((p) => p.key === el<HTMLSelectElement>("st-b").value);
  el("st-msg").textContent = "";
  if (!pa || !pb || pa.key === pb.key) return void (el("st-msg").textContent = "Pick two different places.");
  if (pa.key === "local" && pb.key === "local") return;
  const preset = el<HTMLSelectElement>("st-size").value as Preset;
  const id = `st-${Date.now()}`;
  try {
    const [a, b] = await Promise.all([pa.node(), pb.node()]);
    const { bigMb, smallCount, smallKb } = PRESETS[preset];
    sizeOfRun = PRESETS[preset].label.split(" · ")[0];
    running = id;
    setRunning(true);
    await invoke("speed_test", {
      id,
      a,
      b,
      plan: { bigMb, smallCount, smallKb, both: el<HTMLInputElement>("st-both").checked },
    });
  } catch (e) {
    running = null;
    setRunning(false);
    el("st-msg").textContent = `✗ ${e}`;
  }
}

let sizeOfRun = "";

function setRunning(on: boolean) {
  el("st-progress").hidden = !on;
  el("st-run").textContent = on ? "Cancel" : "Run";
  for (const id of ["st-a", "st-b", "st-size", "st-both", "st-swap"]) el<HTMLInputElement>(id).disabled = on;
}

const mbps = (m: Measure) => (m.secs > 0 ? m.bytes / m.secs : 0);
const fmtSpeed = (bps: number) => `${fmtSize(bps)}/s`;

/** One run's results: a row per route (large file, small files, response time). */
function table(results: Measure[], best: number) {
  const routes = [...new Set(results.filter((m) => m.test !== "latency").map((m) => m.route))];
  const lat = results.filter((m) => m.test === "latency");
  const rows = routes.map((r) => {
    const large = results.find((m) => m.route === r && m.test === "large");
    const small = results.find((m) => m.route === r && m.test === "small");
    const top = large && best > 0 && mbps(large) >= best;
    return `<tr><td>${esc(r)}</td>
      <td class="num${top ? " best" : ""}">${large ? fmtSpeed(mbps(large)) : "…"}${top ? " ★" : ""}</td>
      <td class="num">${small ? `${(small.files / small.secs).toFixed(1)} files/s · ${fmtSpeed(mbps(small))}` : "…"}</td></tr>`;
  });
  const latency = lat.length
    ? `<p class="st-lat">Response time: ${lat.map((m) => `${esc(m.route)} <b>${Math.round(m.secs * 1000)} ms</b>`).join(" · ")}</p>`
    : "";
  return `<table class="st-table"><thead><tr><th>Route</th><th>One large file</th><th>Many small files</th></tr></thead><tbody>${rows.join("")}</tbody></table>${latency}`;
}

/** The current run (if any) above the earlier ones; ★ marks the fastest large-file route. */
function drawResults(current: Measure[] | null) {
  const past = history();
  const all = [...(current ?? []), ...past.flatMap((r) => r.results)];
  const best = Math.max(0, ...all.filter((m) => m.test === "large").map(mbps));
  const when = (t: number) => new Date(t).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
  el("st-results").innerHTML =
    (current?.length ? `<h4>This run</h4>${table(current, best)}` : "") +
    (past.length
      ? `<h4>Earlier runs <button type="button" class="ep-link" data-clear>Clear</button></h4>` +
        past.map((r) => `<p class="st-when">${esc(when(r.at))} · ${esc(r.size)}</p>${table(r.results, best)}`).join("")
      : current
        ? ""
        : `<p class="st-empty">No results yet. Results stay on this computer so you can compare services.</p>`);
}

export function setupSpeedTest() {
  listen<SpeedEvent>("speedtest", (ev) => {
    const e = ev.payload;
    if (e.id !== running) return;
    el("st-phase").textContent = e.phase || "…";
    el<HTMLElement>("st-bar").style.width = e.total ? `${Math.min(100, (e.done / e.total) * 100)}%` : "0%";
    drawResults(e.results);
    if (!e.finished) return;
    running = null;
    setRunning(false);
    if (e.error) el("st-msg").textContent = e.error === "cancelled" ? "Cancelled; test data removed." : `✗ ${e.error}`;
    if (e.results.some((m) => m.test !== "latency")) {
      store.set(
        HISTORY,
        JSON.stringify([{ at: Date.now(), size: sizeOfRun, results: e.results }, ...history()].slice(0, 20)),
      );
      drawResults(null);
    }
  }).catch(fail);
}

/** For ⌘K and menus: is the experiment on and is there a remote to test? */
export const canSpeedTest = () => exp("speed_test") && app.settings.profiles.length > 0;
