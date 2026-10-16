// Transfers UI, driven by `"job"` events from the backend (src-tauri/src/jobs.rs):
// - the activity card (#activity): the current transfer at a glance; click to expand the full list;
// - the transfer beam (#beam): an animated arc from the source pane to the destination's path bar
//   while anything is running, and a pulsing ring on the destination crumb.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { icon } from "./icons";
import { local, remote, remotes } from "./panes";
import type { Pane } from "./pane";
import type { JobEvent } from "./types";
import { $, esc, fail, fmtSize, plural, toast } from "./util";

type JobView = JobEvent & { speed: number; lastBytes: number; lastAt: number };

const jobs = new Map<string, JobView>();
const HIDE_AFTER = 8000; // ms after the last transfer finishes

/** Transfers still running (the updater asks before restarting over them). */
export const activeJobCount = () => [...jobs.values()].filter((j) => j.status === "running").length;

const pct = (j: JobView) => (j.bytes_total ? (j.bytes_done / j.bytes_total) * 100 : j.status === "done" ? 100 : 0);
const whereOf = (j: JobView) => (j.kind === "up" ? `→ ${j.bucket}/${j.prefix}` : `→ ${j.local_dir ?? ""}`);
const numbersOf = (j: JobView) =>
  j.status === "running"
    ? j.files_total
      ? `${fmtSize(j.bytes_done)} / ${fmtSize(j.bytes_total)}${j.speed > 0 ? ` · ${fmtSize(j.speed)}/s` : ""}`
      : "Preparing…"
    : j.status === "done"
      ? `Done · ${plural(j.files_done, "file")}`
      : j.status === "cancelled"
        ? "Cancelled"
        : "Failed";

// ---------- activity card ----------

let hideTimer = 0;

function renderCard() {
  const all = [...jobs.values()].reverse(); // newest first
  const card = $("activity");
  if (!all.length) {
    card.hidden = true;
    document.body.classList.remove("has-activity");
    return;
  }
  const running = all.filter((j) => j.status === "running");
  const focus = running[0] ?? all[0];
  const others = running.length > 1 ? `+${running.length - 1} more` : "";
  const ico =
    focus.status === "running"
      ? icon(focus.kind === "up" ? "arrowUp" : "arrowDown")
      : focus.status === "done"
        ? icon("check")
        : icon("x");
  $("act-top").className = focus.status;
  $("act-top").innerHTML = `
    <span class="act-ico ${focus.status}">${ico}</span>
    <div class="act-main">
      <div class="act-line">
        <span class="act-name">${esc(focus.label)}</span>
        <span class="act-where">${esc(whereOf(focus))}</span>
        <span class="act-num">${esc(numbersOf(focus))}</span>
      </div>
      <div class="bar2"><i style="width:${pct(focus).toFixed(1)}%"></i></div>
    </div>
    ${others ? `<span class="act-more">${others}</span>` : ""}
    ${
      focus.status === "running"
        ? `<button class="ib" data-cancel="${focus.id}" title="Cancel" aria-label="Cancel transfer">${icon("x")}</button>`
        : `<button class="ib" data-dismiss="1" title="Dismiss" aria-label="Dismiss">${icon("x")}</button>`
    }`;
  card.hidden = false;
  document.body.classList.add("has-activity");

  clearTimeout(hideTimer);
  if (!running.length && $("act-list").hidden) {
    hideTimer = window.setTimeout(() => {
      $("activity").hidden = true;
      document.body.classList.remove("has-activity");
    }, HIDE_AFTER);
  }
  if (!$("act-list").hidden) renderList(all);
}

function renderList(all = [...jobs.values()].reverse()) {
  $("jobs-list").innerHTML = all
    .map((j) => {
      const err = j.error ? `<div class="meta err">${esc(j.error)}</div>` : "";
      const ctl =
        j.status === "running"
          ? `<button class="ib" data-cancel="${j.id}" title="Cancel" aria-label="Cancel">${icon("x")}</button>`
          : j.kind === "down" && j.dest && j.files_done
            ? `<button data-reveal="${esc(j.dest)}">Show</button>`
            : "";
      const files = j.files_total ? `${j.files_done}/${j.files_total} files · ` : "";
      return `<div class="job ${j.status}">
        <span class="label">${j.kind === "up" ? "↑" : "↓"} ${esc(j.label)}<small class="where">${esc(whereOf(j))}</small></span>
        <span class="ctl">${ctl}</span>
        <div class="bar2"><i style="width:${pct(j).toFixed(1)}%"></i></div>
        <div class="meta">${files}${esc(numbersOf(j))}</div>${err}
      </div>`;
    })
    .join("");
}

// ---------- transfer beam ----------

function anchorFrom(p: Pane, box: DOMRect, side: "left" | "right") {
  // Prefer the selected row (what the user just sent); else the middle of the list.
  const row = p.list.querySelector<HTMLElement>(".row.sel") ?? null;
  const r = row?.getBoundingClientRect();
  const visible = r && r.bottom > box.top && r.top < box.bottom;
  const src = visible ? r! : p.list.getBoundingClientRect();
  const x = side === "right" ? src.right - 24 : src.left + 24;
  const y = visible ? src.top + src.height / 2 : src.top + Math.min(src.height / 2, 160);
  return { x, y };
}

function anchorTo(p: Pane) {
  const crumb = p.el.querySelector<HTMLElement>(".crumb.last") ?? p.el.querySelector<HTMLElement>(".pathbar")!;
  const r = crumb.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.bottom - 2 };
}

function renderBeam() {
  const beam = $("beam") as unknown as SVGSVGElement;
  const running = [...jobs.values()].filter((j) => j.status === "running");
  document.querySelectorAll(".crumb.target").forEach((c) => c.classList.remove("target"));
  if (!running.length) {
    beam.classList.remove("on");
    return;
  }
  const j = running[running.length - 1]; // most recent
  // Remote side of the job: the view showing its account + bucket (else the focused one).
  const view = (account: string, bucket: string) =>
    remotes.find((v) => v.account === account && v.bucket === bucket) ?? remote;
  const target = view(j.account, j.bucket);
  const from = j.kind === "up" ? local : j.kind === "copy" ? view(j.src_account ?? "", j.src_bucket ?? "") : target;
  const to = j.kind === "down" ? local : target;
  if (from === to) return void beam.classList.remove("on");
  const box = $("panes").getBoundingClientRect();
  const s = anchorFrom(from, from.list.getBoundingClientRect(), from.isRemote ? "left" : "right");
  const e = anchorTo(to);
  const [sx, sy, ex, ey] = [s.x - box.left, s.y - box.top, e.x - box.left, e.y - box.top];
  const dx = (ex - sx) * 0.5;
  const d = `M ${sx} ${sy} C ${sx + dx} ${sy}, ${ex - dx} ${ey + 40}, ${ex} ${ey}`;
  beam.querySelectorAll("path").forEach((p) => {
    p.setAttribute("d", d);
    p.setAttribute("pathLength", "100");
  });
  beam.classList.add("on");
  to.el.querySelector(".crumb.last")?.classList.add("target");
}

// ---------- events ----------

let renderTimer = 0;
function scheduleRender() {
  // setTimeout, not requestAnimationFrame: rAF is paused while the window is hidden/occluded.
  if (!renderTimer)
    renderTimer = window.setTimeout(() => {
      renderTimer = 0;
      renderCard();
      renderBeam();
    }, 60);
}

function onJob(ev: JobEvent) {
  const now = performance.now();
  const prev = jobs.get(ev.id);
  // Smoothed speed from byte deltas, sampled at most every 200 ms.
  let speed = prev?.speed ?? 0;
  if (prev && now - prev.lastAt > 200) {
    const inst = ((ev.bytes_done - prev.lastBytes) * 1000) / (now - prev.lastAt);
    speed = speed ? speed * 0.6 + inst * 0.4 : inst;
  }
  const keepTiming = prev && now - prev.lastAt <= 200;
  jobs.set(ev.id, {
    ...ev,
    speed,
    lastBytes: keepTiming ? prev!.lastBytes : ev.bytes_done,
    lastAt: keepTiming ? prev!.lastAt : now,
  });
  if (ev.status !== "running") {
    // Refresh whichever pane is showing the destination.
    // Refresh whichever views are showing the destination.
    if (ev.kind !== "down")
      for (const v of remotes)
        if (v.account === ev.account && v.bucket === ev.bucket && ev.prefix.startsWith(v.cwd)) v.load(true);
    if (ev.kind === "down" && ev.local_dir?.startsWith(local.cwd)) local.load(true);
    const verb = ev.kind === "up" ? "Uploaded" : ev.kind === "copy" ? "Copied" : "Downloaded";
    if (ev.status === "done") toast(`${verb}: ${ev.label}`);
  }
  scheduleRender();
}

/** A new remote view: keep the beam on its rows while it scrolls. */
export function watchView(v: Pane) {
  v.list.addEventListener("scroll", scheduleRender, { passive: true });
}

export async function setupJobs() {
  $("activity").addEventListener("click", (ev) => {
    const t = (ev.target as HTMLElement).closest<HTMLElement>("button");
    if (t?.dataset.cancel) return void invoke("cancel", { id: t.dataset.cancel });
    if (t?.dataset.reveal) return void invoke("reveal", { path: t.dataset.reveal }).catch(fail);
    if (t?.dataset.dismiss) {
      for (const [id, j] of jobs) if (j.status !== "running") jobs.delete(id);
      $("act-list").hidden = true;
      return scheduleRender();
    }
    if (t?.id === "jobs-clear") {
      for (const [id, j] of jobs) if (j.status !== "running") jobs.delete(id);
      return scheduleRender();
    }
    if ((ev.target as HTMLElement).closest("#act-top")) {
      $("act-list").hidden = !$("act-list").hidden;
      scheduleRender();
    }
  });
  window.addEventListener("resize", scheduleRender);
  local.list.addEventListener("scroll", scheduleRender, { passive: true });
  await listen<JobEvent>("job", (e) => onJob(e.payload));
}
