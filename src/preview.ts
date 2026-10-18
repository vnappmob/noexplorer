// Quick-look modal. Remote media is streamed via a presigned URL; local media via the
// nxr:// scheme (`localSrc`, proxy.rs); text is read (first 512 KB) through a backend command.

import { invoke } from "@tauri-apps/api/core";
import { ext, ICON, kindOf, localSrc } from "./files";
import type { Pane } from "./pane";
import { active } from "./panes";
import type { Item } from "./types";
import { $, esc, fmtDate, fmtSize } from "./util";

const TEXT_MAX = 512 * 1024;

/** What's open in the preview, or null when closed. */
export let pv: { pane: Pane; key: string } | null = null;

export async function openPreview(p: Pane, e: Item) {
  pv = { pane: p, key: e.key };
  const same = () => pv?.key === e.key; // a newer preview may have replaced this one
  $("preview").hidden = false;
  $("pv-title").textContent = `${e.name} · ${fmtSize(e.size)}`;
  $("pv-transfer").textContent = p.isRemote ? "← Download" : "Upload →";
  const body = $("pv-body");
  body.innerHTML = `<div class="note">Loading…</div>`;
  const k = kindOf(e.name);
  try {
    if (k === "text" || (k === "other" && e.size < 64 * 1024 && !ext(e.name))) {
      const txt = p.isRemote
        ? await invoke<string>("read_text", { ...p.where(), key: e.key, max: TEXT_MAX })
        : await invoke<string>("read_local_text", { path: e.key, max: TEXT_MAX });
      if (!same()) return;
      body.innerHTML = `<pre></pre>`;
      body.querySelector("pre")!.textContent =
        txt + (e.size > TEXT_MAX ? `\n\n… (showing the first ${fmtSize(TEXT_MAX)} of ${fmtSize(e.size)})` : "");
      return;
    }
    if (k === "image" || k === "video" || k === "audio" || k === "pdf") {
      const url = p.isRemote
        ? await invoke<string>("presign", { ...p.where(), key: e.key, secs: 3600, inline: true })
        : localSrc(e.key);
      if (!same()) return;
      const u = esc(url);
      body.innerHTML =
        k === "image"
          ? `<img src="${u}" alt="" />`
          : k === "video"
            ? `<video src="${u}" controls autoplay></video>`
            : k === "audio"
              ? `<audio src="${u}" controls autoplay></audio>`
              : `<iframe src="${u}"></iframe>`;
      body.querySelector("img,video,audio")?.addEventListener("error", () => {
        if (same()) body.innerHTML = noPreview(e, "This format can't be shown in the app.");
      });
      return;
    }
    body.innerHTML = noPreview(e, "No quick preview for this file type.");
  } catch (err) {
    if (same()) body.innerHTML = noPreview(e, esc(String(err)));
  }
}

function noPreview(e: Item, why: string) {
  return `<div class="note"><div style="font-size:48px">${ICON[kindOf(e.name)]}</div>${why}<br/>
    Click <b>↗ Open</b> to open it with the default app.<br/>
    <small>${esc(e.name)} · ${fmtSize(e.size)} · ${fmtDate(e.modified)}</small></div>`;
}

export function closePreview() {
  pv = null;
  $("preview").hidden = true;
  $("pv-body").innerHTML = ""; // stops media playback
  active.list.focus();
}

/** The item currently previewed. */
export const pvItem = () => (pv ? pv.pane.view.find((e) => e.key === pv!.key) : undefined);

/** Previous / next file in the same pane (←/→ in the preview). */
export function previewStep(d: number) {
  if (!pv) return;
  const p = pv.pane;
  const files = p.view.filter((e) => !e.dir);
  const next = files[files.findIndex((e) => e.key === pv!.key) + d];
  if (!next) return;
  p.selected = new Set([next.key]);
  p.anchor = p.view.indexOf(next);
  p.paint();
  openPreview(p, next);
}
