// Transfers UI, driven by `"job"` events from the backend (src-tauri/src/jobs.rs):
// - the activity card (#activity): the current transfer at a glance; click to expand the full list;
// - the transfer beam (#beam): an animated arc from the source pane to the destination's path bar
//   while anything is running, and a pulsing ring on the destination crumb.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { icon } from "./icons";
import { local, remotes } from "./panes";
import { openRemote } from "./settings";
import { app, exp } from "./state";
import type { Pane } from "./pane";
import type { JobEvent, PendingJob } from "./types";
import { $, esc, fail, fmtSize, plural, toast } from "./util";

type JobView = JobEvent & { speed: number; lastBytes: number; lastAt: number };

const jobs = new Map<string, JobView>();
const HIDE_AFTER = 8000; // ms after the last transfer finishes

/** Transfers still running (the updater asks before restarting over them). */
export const activeJobCount = () => [...jobs.values()].filter((j) => j.status === "running").length;

/** Jobs that work on items inside one remote (delete, move, duplicate): counted in items, not
 * bytes, and nothing moves between panes (no beam, no lane). */
const ITEM_JOBS: Record<string, [doing: string, done: string]> = {
  delete: ["Deleting", "Deleted"],
  move: ["Moving", "Moved"],
  dup: ["Duplicating", "Duplicated"],
};
const isItemJob = (j: { kind: string }) => j.kind in ITEM_JOBS;

const pct = (j: JobView) => {
  const [done, total] = isItemJob(j) ? [j.files_done, j.files_total] : [j.bytes_done, j.bytes_total];
  return total ? (done / total) * 100 : j.status === "done" ? 100 : 0;
};
const whereOf = (j: JobView) =>
  isItemJob(j)
    ? `in ${j.bucket}/${j.prefix}`
    : j.kind === "down"
      ? `→ ${j.local_dir ?? ""}`
      : `→ ${j.bucket}/${j.prefix}`;
const ICON: Record<string, string> = {
  up: "arrowUp",
  down: "arrowDown",
  copy: "arrowRight",
  delete: "trash",
  move: "folder",
  dup: "copy",
};
const GLYPH: Record<string, string> = { up: "↑", down: "↓", copy: "→", delete: "×", move: "↪", dup: "⧉" };
const numbersOf = (j: JobView) => {
  if (isItemJob(j)) {
    const [doing, done] = ITEM_JOBS[j.kind];
    if (j.status === "running")
      return j.files_total ? `${doing} · ${j.files_done} / ${plural(j.files_total, "item")}` : "Finding items…";
    if (j.status === "done") return `${done} · ${plural(j.files_total, "item")}`;
    return j.status === "cancelled" ? `Stopped · ${j.files_done} done` : "Failed";
  }
  if (j.status === "running" && j.paused) return `Paused · ${fmtSize(j.bytes_done)} / ${fmtSize(j.bytes_total)}`;
  return j.status === "running"
    ? j.files_total
      ? `${fmtSize(j.bytes_done)} / ${fmtSize(j.bytes_total)}${j.speed > 0 ? ` · ${fmtSize(j.speed)}/s` : ""}`
      : "Preparing…"
    : j.status === "done"
      ? `Done · ${plural(j.files_done, "file")}`
      : j.status === "cancelled"
        ? "Cancelled"
        : "Failed";
};

/** Pause / resume for a running transfer (experimental resume). */
const pauseBtn = (j: JobView) =>
  exp("resume") && j.status === "running" && !isItemJob(j)
    ? `<button class="ib" data-pause="${j.id}" data-on="${!j.paused}" title="${j.paused ? "Resume" : "Pause"}" aria-label="${j.paused ? "Resume" : "Pause"} transfer">${icon(j.paused ? "play" : "pause")}</button>`
    : "";

// ---------- interrupted transfers (experimental resume) ----------

let interrupted: PendingJob[] = [];

function renderResume() {
  const box = $("act-resume");
  box.hidden = !interrupted.length;
  if (!interrupted.length) return;
  const n = interrupted.length;
  box.innerHTML = `<div class="resume-text"><b>${n === 1 ? "A transfer" : `${n} transfers`} didn't finish</b>
      <small>${esc(interrupted.map((p) => p.label).join(", "))}</small></div>
    <button data-resume-all="1" class="primary">Resume</button>
    <button data-resume-dismiss="1">Dismiss</button>`;
}

/** At startup: offer to resume transfers the app was closed (or crashed) in the middle of. */
export async function checkInterrupted() {
  if (!exp("resume")) return;
  interrupted = await invoke<PendingJob[]>("interrupted_jobs").catch(() => []);
  scheduleRender();
}

// ---------- activity card ----------

let hideTimer = 0;

function renderCard() {
  const all = [...jobs.values()].reverse(); // newest first
  const card = $("activity");
  renderResume();
  $("act-top").hidden = !all.length;
  if (!all.length) {
    card.hidden = !interrupted.length;
    document.body.classList.toggle("has-activity", !!interrupted.length);
    return;
  }
  const running = all.filter((j) => j.status === "running");
  const focus = running[0] ?? all[0];
  const others = running.length > 1 ? `+${running.length - 1} more` : "";
  const ico =
    focus.status === "running"
      ? icon(ICON[focus.kind] ?? "arrowDown")
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
    ${pauseBtn(focus)}${
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
          ? `${pauseBtn(j)}<button class="ib" data-cancel="${j.id}" title="Cancel" aria-label="Cancel">${icon("x")}</button>`
          : j.kind === "down" && j.dest && j.files_done
            ? `<button data-reveal="${esc(j.dest)}">Show</button>`
            : "";
      const files = j.files_total ? `${j.files_done}/${j.files_total} files · ` : "";
      return `<div class="job ${j.status}">
        <span class="label">${GLYPH[j.kind] ?? "↓"} ${esc(j.label)}<small class="where">${esc(whereOf(j))}</small></span>
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
const same = (a: string | null | undefined, b: string | null | undefined) => trimSlash(a ?? "") === trimSlash(b ?? "");

/** Keys of the items each transfer carries (from `transfer()` in actions.ts): the beam starts at
 * them, never at whatever happens to be selected now. Jobs started elsewhere (Compare) have none. */
const jobItems = new Map<string, Set<string>>();

export function noteJobItems(id: string, keys: string[]) {
  const j = jobs.get(id);
  if (j && j.status !== "running") {
    // Already over (a quick delete can finish before its id comes back): un-dim right away.
    if (j.kind === "delete") for (const v of remotes) keys.forEach((k) => v.pending.delete(k));
    return void remotes.forEach((v) => v.render());
  }
  jobItems.set(id, new Set(keys));
  scheduleRender();
}

/**
 * Start of the beam in `p`: the first of the job's rows that's in view. When they're scrolled
 * away, the list edge on their side (top or bottom); when unknown, the middle of the list.
 */
function anchorFrom(p: Pane, side: "left" | "right", keys: Set<string> | undefined): Pt {
  const box = p.list.getBoundingClientRect();
  const x = (r: DOMRect) => (side === "right" ? r.right - 24 : r.left + 24);
  const middle = { x: x(box), y: box.top + Math.min(box.height / 2, 160) };
  if (!keys?.size) return middle;
  const ids = p.view.flatMap((e, i) => (keys.has(e.key) ? [i] : []));
  if (!ids.length) return middle;
  // Rows in the DOM that are inside the list's viewport (the list is virtualized).
  const shown = [...p.list.querySelectorAll<HTMLElement>(".row")]
    .map((el) => ({ i: Number(el.dataset.i), r: el.getBoundingClientRect() }))
    .filter(({ r }) => r.bottom > box.top + 2 && r.top < box.bottom - 2);
  const hit = shown.find(({ i }) => ids.includes(i));
  if (hit) return { x: x(hit.r), y: hit.r.top + hit.r.height / 2 };
  const first = shown.length ? Math.min(...shown.map((s) => s.i)) : 0;
  return { x: x(box), y: ids[0] < first ? box.top + 6 : box.bottom - 6 };
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

/** `dir` is `cwd` or inside it. */
function under(dir: string, cwd: string) {
  const d = trimSlash(dir);
  const c = trimSlash(cwd);
  return !c || d === c || d.startsWith(c + "/") || d.startsWith(c + "\\");
}

/**
 * Where the beam goes for job `j`, or null when the panes have left its folders (then it gets a
 * lane instead, never both): the source pane must show the source folder, and the destination pane
 * the destination folder or a folder above it (the beam then aims at the row leading into it).
 */
function placement(j: JobView): { from: Pane; to: Pane; dir: string } | null {
  if (isItemJob(j)) return null;
  let from: Pane | null, to: Pane | null, srcDir: string | null, dstDir: string | null;
  if (j.kind === "up") [from, to, srcDir, dstDir] = [local, viewFor(j.account, j.bucket), j.src_dir, j.prefix];
  else if (j.kind === "down") [from, to, srcDir, dstDir] = [viewFor(j.account, j.bucket), local, j.prefix, j.local_dir];
  else
    [from, to, srcDir, dstDir] = [
      viewFor(j.src_account, j.src_bucket),
      viewFor(j.account, j.bucket),
      j.src_dir,
      j.prefix,
    ];
  if (!from || !to || from === to || srcDir == null || dstDir == null) return null;
  if (!same(from.cwd, srcDir) || !under(dstDir, to.cwd)) return null;
  return { from, to, dir: dstDir };
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
  const job = running.find((j) => placement(j));
  const ends = job && placement(job);
  if (!job || !ends) {
    beam.classList.remove("on"); // fades out where it was (no jump while fading)
    return;
  }
  const box = $("panes").getBoundingClientRect();
  const s = anchorFrom(ends.from, ends.from.isRemote ? "left" : "right", jobItems.get(job.id));
  const e = anchorTo(ends.to, ends.dir);
  moveBeam([s.x - box.left, s.y - box.top, e.at.x - box.left, e.at.y - box.top], wasOn);
  beam.classList.add("on");
  if (e.exact) e.el.classList.add("target");
}

// ---------- transfer lanes ----------
//
// While a transfer runs and the panes have left its folders (`placement` is null), a lane above the
// panes keeps showing it: two boxes lined up with the two panes, each with that side's full path,
// and the transfer flowing between them over the gap. One lane per such transfer. Clicking a box
// opens that folder in its pane; clicking the middle opens both (and the lane goes away).

const SCHEME: Record<string, string> = { s3: "s3://", nextcloud: "nextcloud://", webdav: "dav://", box: "box://" };

type Side = { icon: string; name: string; path: string; open: () => unknown };

function localSide(dir: string | null): Side {
  return { icon: "desktop", name: "", path: dir ?? "", open: () => dir && local.go(dir) };
}

function remoteSide(account: string | null, bucket: string | null, prefix: string | null): Side {
  const p = app.settings.profiles.find((x) => x.id === account);
  return {
    icon: "cloud",
    name: p?.name ?? "Remote",
    // The same form as the pane's path bar: scheme + container + folders.
    path: `${SCHEME[p?.kind ?? "s3"] ?? ""}${bucket ?? ""}/${prefix ?? ""}`,
    open: () => account && bucket != null && openRemote(account, bucket, prefix ?? ""),
  };
}

/** Left box = over the left pane. Uploads and copies flow left → right, downloads right → left. */
function sides(j: JobView): { left: Side; right: Side; rtl: boolean } {
  const dst = remoteSide(j.account, j.bucket, j.prefix);
  if (j.kind === "up") return { left: localSide(j.src_dir), right: dst, rtl: false };
  if (j.kind === "down") return { left: localSide(j.local_dir), right: dst, rtl: true };
  return { left: remoteSide(j.src_account, j.src_bucket, j.src_dir), right: dst, rtl: false };
}

/** The box's content; the path is clipped at its start, so the deepest folders stay visible. */
const boxHtml = (s: Side) =>
  `${icon(s.icon)}${s.name ? `<b>${esc(s.name)}</b>` : ""}<span class="lane-path"><bdi>${esc(s.path)}</bdi></span>`;

const laneEls = new Map<string, HTMLElement>();

function renderLanes() {
  const box = $("lanes");
  const show = [...jobs.values()].filter((j) => j.status === "running" && !isItemJob(j) && !placement(j));
  const keep = new Set(show.map((j) => j.id));
  for (const [id, el] of laneEls)
    if (!keep.has(id)) {
      // Slide out, so the panes glide back up instead of jumping.
      laneEls.delete(id);
      el.classList.add("out");
      setTimeout(() => el.remove(), 220);
    }
  for (const j of show) {
    const s = sides(j);
    let el = laneEls.get(j.id);
    if (!el) {
      // Built once and updated in place, so a click isn't lost to a re-render mid-press.
      el = document.createElement("div");
      el.className = "lane";
      el.innerHTML = `<button class="lane-box l"></button>
        <button class="lane-mid" title="Show both folders">
          <svg class="lane-flow" viewBox="0 0 48 12" preserveAspectRatio="none" aria-hidden="true">
            <path class="track" d="M2 6 H46" /><path class="dash" d="M2 6 H46" /><path class="head" d="" />
          </svg>
          <span class="lane-num"></span>
        </button>
        <span class="lane-box r"><button class="lane-open"></button>
          <button class="ib lane-x" title="Cancel" aria-label="Cancel transfer">${icon("x")}</button>
          <i class="lane-bar"></i></span>`;
      const id = j.id;
      const cur = () => sides(jobs.get(id) ?? j);
      el.querySelector<HTMLElement>(".lane-box.l")!.onclick = () => cur().left.open();
      el.querySelector<HTMLElement>(".lane-open")!.onclick = () => cur().right.open();
      el.querySelector<HTMLElement>(".lane-mid")!.onclick = () => (cur().left.open(), cur().right.open());
      el.querySelector<HTMLElement>(".lane-x")!.onclick = () => invoke("cancel", { id });
      box.append(el);
      laneEls.set(j.id, el);
    }
    const l = el.querySelector<HTMLElement>(".lane-box.l")!;
    const r = el.querySelector<HTMLElement>(".lane-open")!;
    if (l.dataset.path !== s.left.path) {
      l.innerHTML = boxHtml(s.left);
      l.dataset.path = s.left.path;
      l.title = `Show ${s.left.path}`;
    }
    if (r.dataset.path !== s.right.path) {
      r.innerHTML = boxHtml(s.right);
      r.dataset.path = s.right.path;
      r.title = `Show ${s.right.path}`;
    }
    el.classList.toggle("rtl", s.rtl);
    el.querySelector<SVGPathElement>(".head")!.setAttribute("d", s.rtl ? "M7 2 L2 6 L7 10" : "M41 2 L46 6 L41 10");
    el.querySelector(".lane-num")!.textContent = j.files_total ? `${Math.floor(pct(j))}%` : "…";
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
    // A finished delete un-dims only its own rows (the refresh below drops the deleted ones).
    if (ev.kind === "delete") for (const v of remotes) jobItems.get(ev.id)?.forEach((k) => v.pending.delete(k));
    jobItems.delete(ev.id);
    // Refresh the views showing the destination. A move changes two folders (source and target),
    // so item jobs refresh every view on that bucket.
    if (ev.kind !== "down")
      for (const v of remotes)
        if (v.account === ev.account && v.bucket === ev.bucket && (isItemJob(ev) || ev.prefix.startsWith(v.cwd)))
          v.load(true);
    if (ev.kind === "down" && ev.local_dir?.startsWith(local.cwd)) local.load(true);
    const verbs: Record<string, string> = { up: "Uploaded", copy: "Copied", down: "Downloaded" };
    const verb = ITEM_JOBS[ev.kind]?.[1] ?? verbs[ev.kind];
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
    if (t?.dataset.pause) return void invoke("pause", { id: t.dataset.pause, on: t.dataset.on === "true" });
    if (t?.dataset.resumeAll) {
      const all = interrupted;
      interrupted = [];
      scheduleRender();
      return void Promise.all(all.map((p) => invoke("resume_job", { id: p.id }))).catch(fail);
    }
    if (t?.dataset.resumeDismiss) {
      const all = interrupted;
      interrupted = [];
      scheduleRender();
      return void all.forEach((p) => invoke("dismiss_interrupted", { id: p.id }));
    }
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
