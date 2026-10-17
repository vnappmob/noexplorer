// Transfers UI, driven by `"job"` events from the backend (src-tauri/src/jobs.rs):
// - the activity card (#activity): the current transfer at a glance; click to expand the full list;
// - the transfer beam (#beam): an animated arc from the source pane to the destination's path bar
//   while anything is running, and a pulsing ring on the destination crumb.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { icon } from "./icons";
import { local, remotes } from "./panes";
import { openRemote } from "./settings";
import { app } from "./state";
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
//
// The beam only connects things that are on screen: the source and destination must each be shown
// by a pane (a remote view on the job's account + bucket, not connecting or failed). Otherwise it
// fades out, rather than pointing into a view that now shows another account. Its ends glide to new
// positions instead of jumping when panes scroll, switch or reload.

type Pt = { x: number; y: number };

/** The remote view showing `account` + `bucket` and ready (not connecting / failed), if any. */
const viewFor = (account: string | null | undefined, bucket: string | null | undefined) =>
  remotes.find((v) => v.account === account && v.bucket === bucket && !v.notice) ?? null;

const trimSlash = (p: string) => p.replace(/[\\/]+$/, "");

function anchorFrom(p: Pane, side: "left" | "right"): Pt {
  // The selected row (what the user just sent) when it's in view; else the middle of the list.
  const box = p.list.getBoundingClientRect();
  const r = p.list.querySelector<HTMLElement>(".row.sel")?.getBoundingClientRect();
  const visible = r && r.bottom > box.top && r.top < box.bottom;
  const src = visible ? r! : box;
  return {
    x: side === "right" ? src.right - 24 : src.left + 24,
    y: visible ? src.top + src.height / 2 : src.top + Math.min(src.height / 2, 160),
  };
}

/**
 * Where the files land in `p`: the crumb of `dir` when the pane is in it or below it, the row of
 * the subfolder leading to it when the pane is above it, else the pane's first crumb (the bucket or
 * disk). `exact` = it's the destination folder itself (it gets the pulsing ring).
 */
function anchorTo(p: Pane, dir: string): { at: Pt; el: HTMLElement; exact: boolean } {
  const want = trimSlash(dir);
  const crumbs = [...p.el.querySelectorAll<HTMLElement>(".crumb")];
  let el = crumbs.find((c) => trimSlash(c.dataset.p ?? "") === want);
  let exact = !!el;
  if (!el) {
    const here = trimSlash(p.cwd);
    const sep = p.isRemote ? "/" : want.includes("\\") ? "\\" : "/";
    if (want.startsWith(here ? here + sep : "")) {
      const next = (here ? here + sep : "") + want.slice(here ? here.length + 1 : 0).split(sep)[0];
      el =
        [...p.list.querySelectorAll<HTMLElement>(".row[data-dir]")].find(
          (r) => trimSlash(r.dataset.dir ?? "") === next,
        ) ?? undefined;
    }
  }
  el ??= crumbs[0] ?? p.q(".pathbar");
  const r = el.getBoundingClientRect();
  const isRow = el.classList.contains("row");
  return {
    at: isRow ? { x: r.left + 40, y: r.top + r.height / 2 } : { x: r.left + r.width / 2, y: r.bottom - 2 },
    el,
    exact,
  };
}

/** The ends of a job's beam, or null when one of them isn't on screen. */
function endsOf(j: JobView): { from: Pane; to: Pane; dir: string } | null {
  if (j.kind === "up") {
    const to = viewFor(j.account, j.bucket);
    return to ? { from: local, to, dir: j.prefix } : null;
  }
  if (j.kind === "down") {
    const from = viewFor(j.account, j.bucket);
    return from && j.local_dir ? { from, to: local, dir: j.local_dir } : null;
  }
  const from = viewFor(j.src_account, j.src_bucket);
  const to = viewFor(j.account, j.bucket);
  return from && to && from !== to ? { from, to, dir: j.prefix } : null;
}

// Current beam ends (relative to #panes) and the glide towards new ones.
let cur: [number, number, number, number] | null = null;
let glide = 0;

function drawPath([sx, sy, ex, ey]: [number, number, number, number]) {
  const dx = (ex - sx) * 0.5;
  const d = `M ${sx} ${sy} C ${sx + dx} ${sy}, ${ex - dx} ${ey + 40}, ${ex} ${ey}`;
  $("beam")
    .querySelectorAll("path")
    .forEach((p) => {
      p.setAttribute("d", d);
      p.setAttribute("pathLength", "100");
    });
}

/** Moves the beam to `to` over ~200 ms (or at once when it was hidden or barely moves). */
function moveBeam(to: [number, number, number, number], wasOn: boolean) {
  clearInterval(glide);
  const from = cur;
  cur = to;
  const far = from && Math.max(...to.map((v, i) => Math.abs(v - from[i]))) > 4;
  if (!wasOn || !from || !far) return drawPath(to);
  // setInterval, not rAF: rAF pauses in a covered window, which would freeze the beam mid-glide.
  const t0 = performance.now();
  glide = window.setInterval(() => {
    const k = Math.min(1, (performance.now() - t0) / 200);
    const e = 1 - (1 - k) ** 3; // ease-out
    drawPath(from.map((v, i) => v + (to[i] - v) * e) as [number, number, number, number]);
    if (k >= 1) clearInterval(glide);
  }, 16);
}

function renderBeam() {
  const beam = $("beam") as unknown as SVGSVGElement;
  const wasOn = beam.classList.contains("on");
  document.querySelectorAll(".crumb.target").forEach((c) => c.classList.remove("target"));
  // The newest running job whose two ends are on screen.
  const running = [...jobs.values()].filter((j) => j.status === "running").reverse();
  const ends = running.map(endsOf).find((e) => e) ?? null;
  if (!ends) {
    beam.classList.remove("on"); // fades out where it was (no jump while fading)
    return;
  }
  const box = $("panes").getBoundingClientRect();
  const s = anchorFrom(ends.from, ends.from.isRemote ? "left" : "right");
  const e = anchorTo(ends.to, ends.dir);
  moveBeam([s.x - box.left, s.y - box.top, e.at.x - box.left, e.at.y - box.top], wasOn);
  beam.classList.add("on");
  if (e.exact) e.el.classList.add("target");
}

// ---------- transfer lanes ----------
//
// While a transfer runs and the panes have moved away from its folders, a lane above the panes
// keeps saying what goes where: the local side over the left pane, the remote side over the right
// one, the arrow pointing the way the files move. One lane per such transfer. Clicking a side opens
// that folder in its pane; clicking the lane opens both, and the lane goes away.

const same = (a: string | null | undefined, b: string | null | undefined) => trimSlash(a ?? "") === trimSlash(b ?? "");

/** Both panes show exactly this job's folders (the beam covers it, no lane needed). */
function inPlace(j: JobView) {
  const dst = viewFor(j.account, j.bucket);
  if (j.kind === "up") return same(local.cwd, j.src_dir) && !!dst && same(dst.cwd, j.prefix);
  if (j.kind === "down") return !!dst && same(dst.cwd, j.prefix) && same(local.cwd, j.local_dir);
  const src = viewFor(j.src_account, j.src_bucket);
  return !!src && !!dst && same(src.cwd, j.src_dir) && same(dst.cwd, j.prefix);
}

/** `…/Projects/website` (last segments, home as ~). */
function shortLocal(path: string) {
  const home = local.roots.find((r) => /Home$/.test(r.name))?.path;
  let p = home && path.startsWith(home) ? "~" + path.slice(home.length) : path;
  const parts = p.split(/[\\/]/).filter(Boolean);
  if (parts.length > 3) p = "…/" + parts.slice(-3).join("/");
  return p || "/";
}

function remoteLabel(account: string | null, bucket: string | null, prefix: string | null) {
  const p = app.settings.profiles.find((x) => x.id === account);
  const where = p?.kind === "s3" ? `${bucket ?? ""}/${prefix ?? ""}` : `/${prefix ?? ""}`;
  return { name: p?.name ?? "Remote", where };
}

type Side = { html: string; title: string; open: () => unknown };

function sides(j: JobView): { left: Side; right: Side; arrow: string } {
  const loc = (dir: string | null): Side => ({
    html: `${icon("desktop")}<span>${esc(shortLocal(dir ?? ""))}</span>`,
    title: dir ?? "",
    open: () => dir && local.go(dir),
  });
  const rem = (account: string | null, bucket: string | null, prefix: string | null): Side => {
    const r = remoteLabel(account, bucket, prefix);
    return {
      html: `${icon("cloud")}<b>${esc(r.name)}</b><span>${esc(r.where)}</span>`,
      title: `${r.name} · ${r.where}`,
      open: () => account && bucket != null && openRemote(account, bucket, prefix ?? ""),
    };
  };
  const dst = rem(j.account, j.bucket, j.prefix);
  if (j.kind === "up") return { left: loc(j.src_dir), right: dst, arrow: "→" };
  if (j.kind === "down") return { left: loc(j.local_dir), right: dst, arrow: "←" };
  return { left: rem(j.src_account, j.src_bucket, j.src_dir), right: dst, arrow: "→" };
}

const laneEls = new Map<string, HTMLElement>();

function renderLanes() {
  const box = $("lanes");
  const show = [...jobs.values()].filter((j) => j.status === "running" && !inPlace(j));
  const keep = new Set(show.map((j) => j.id));
  for (const [id, el] of laneEls)
    if (!keep.has(id)) {
      // Slide out, so the panes glide back up instead of jumping.
      laneEls.delete(id);
      el.classList.add("out");
      setTimeout(() => el.remove(), 220);
    }
  for (const j of show) {
    let el = laneEls.get(j.id);
    const s = sides(j);
    if (!el) {
      // Built once and updated in place, so a click isn't lost to a re-render mid-press.
      el = document.createElement("div");
      el.className = "lane";
      el.innerHTML = `<button class="lane-side l"></button><span class="lane-arrow"></span>
        <span class="lane-r"><button class="lane-side r"></button><span class="lane-num"></span>
        <button class="ib lane-x" title="Cancel" aria-label="Cancel transfer">${icon("x")}</button></span>
        <i class="lane-bar"></i>`;
      el.querySelector<HTMLElement>(".lane-x")!.onclick = (ev) => {
        ev.stopPropagation();
        invoke("cancel", { id: j.id });
      };
      el.onclick = (ev) => {
        const side = (ev.target as HTMLElement).closest(".lane-side");
        const cur = sides(jobs.get(j.id) ?? j);
        if (!side || side.classList.contains("l")) cur.left.open();
        if (!side || side.classList.contains("r")) cur.right.open();
      };
      box.append(el);
      laneEls.set(j.id, el);
    }
    const l = el.querySelector<HTMLElement>(".lane-side.l")!;
    const r = el.querySelector<HTMLElement>(".lane-side.r")!;
    if (l.title !== s.left.title) ((l.innerHTML = s.left.html), (l.title = `Show ${s.left.title}`));
    if (r.title !== `Show ${s.right.title}`) ((r.innerHTML = s.right.html), (r.title = `Show ${s.right.title}`));
    el.querySelector(".lane-arrow")!.textContent = s.arrow;
    el.querySelector(".lane-num")!.textContent = j.files_total
      ? `${Math.floor(pct(j))}% · ${j.files_done}/${j.files_total}`
      : "Preparing…";
    el.querySelector<HTMLElement>(".lane-bar")!.style.width = `${pct(j).toFixed(1)}%`;
  }
}

// ---------- events ----------

let renderTimer = 0;
function scheduleRender() {
  // setTimeout, not requestAnimationFrame: rAF is paused while the window is hidden/occluded.
  if (!renderTimer)
    renderTimer = window.setTimeout(() => {
      renderTimer = 0;
      renderCard();
      renderLanes();
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
  // A pane re-rendered (switched account, reloaded, scrolled to new rows): re-aim the beam.
  document.addEventListener("panechange", () => activeJobCount() && scheduleRender());
  local.list.addEventListener("scroll", scheduleRender, { passive: true });
  await listen<JobEvent>("job", (e) => onJob(e.payload));
}
