// Transfers panel: renders `"job"` events from the backend (see src-tauri/src/jobs.rs).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { local, remote } from "./panes";
import { app } from "./state";
import type { JobEvent } from "./types";
import { $, esc, fail, fmtSize, plural, toast } from "./util";

type JobView = JobEvent & { speed: number; lastBytes: number; lastAt: number };

const jobs = new Map<string, JobView>();

function renderJobs() {
  const list = [...jobs.values()].reverse();
  $("jobs").hidden = !list.length;
  $("jobs-list").innerHTML = list
    .map((j) => {
      const pct = j.bytes_total ? (j.bytes_done / j.bytes_total) * 100 : j.status === "done" ? 100 : 0;
      const where = j.kind === "up" ? `→ ${j.bucket}/${j.prefix}` : `→ ${j.local_dir ?? ""}`;
      let meta = "";
      if (j.status === "running") {
        meta = j.files_total
          ? `${j.files_done}/${j.files_total} files · ${fmtSize(j.bytes_done)} / ${fmtSize(j.bytes_total)}${
              j.speed > 0 ? ` · ${fmtSize(j.speed)}/s` : ""
            }`
          : "Preparing…";
      } else if (j.status === "done") {
        meta = `Done · ${plural(j.files_done, "file")} · ${fmtSize(j.bytes_total)}`;
      } else if (j.status === "cancelled") {
        meta = `Cancelled · ${j.files_done}/${j.files_total} files`;
      }
      const err = j.error ? `<div class="meta err">${esc(j.error)}</div>` : "";
      const ctl =
        j.status === "running"
          ? `<button class="icon" data-cancel="${j.id}" title="Cancel">✕</button>`
          : j.kind === "down" && j.dest && j.files_done
            ? `<button data-reveal="${esc(j.dest)}">Show</button>`
            : "";
      return `<div class="job ${j.status}">
        <span>${j.kind === "up" ? "⬆" : "⬇"}</span>
        <span class="label">${esc(j.label)} <small class="where">${esc(where)}</small></span>
        <span class="ctl">${ctl}</span>
        <div class="bar2"><i style="width:${pct.toFixed(1)}%"></i></div>
        <div class="meta">${meta}</div>${err}
      </div>`;
    })
    .join("");
}

let renderTimer = 0;
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
    if (ev.kind === "up" && ev.bucket === app.bucket && ev.prefix.startsWith(remote.cwd)) remote.load(true);
    if (ev.kind === "down" && ev.local_dir?.startsWith(local.cwd)) local.load(true);
    if (ev.status === "done") toast(`${ev.kind === "up" ? "Uploaded" : "Downloaded"}: ${ev.label}`);
  }
  // setTimeout, not requestAnimationFrame: rAF is paused while the window is hidden/occluded.
  if (!renderTimer) renderTimer = window.setTimeout(() => ((renderTimer = 0), renderJobs()), 60);
}

export async function setupJobs() {
  $("jobs-list").addEventListener("click", (ev) => {
    const t = (ev.target as HTMLElement).closest<HTMLElement>("button");
    if (t?.dataset.cancel) invoke("cancel", { id: t.dataset.cancel });
    if (t?.dataset.reveal) invoke("reveal", { path: t.dataset.reveal }).catch(fail);
  });
  $("jobs-clear").onclick = () => {
    for (const [id, j] of jobs) if (j.status !== "running") jobs.delete(id);
    renderJobs();
  };
  await listen<JobEvent>("job", (e) => onJob(e.payload));
}
