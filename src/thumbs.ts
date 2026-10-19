// Grid view thumbnails. Images show a small thumbnail that the backend makes
// and caches (thumbs.rs: 16 MB in memory, 48 MB on disk, `nxr://localhost/~thumb/…`); videos show
// their frame one second in, through the same link quick look uses. Only tiles near the viewport
// exist (the grid is virtualized), so:
// - loaded pictures are kept (`kept`, a small LRU of elements) and moved into the new tiles when
//   the grid redraws, so scrolling never blanks a picture that was already showing;
// - the next screen's thumbnails are requested ahead (`prefetch`), so they're cached when it comes.

import { invoke } from "@tauri-apps/api/core";
import { kindOf, localSrc } from "./files";
import type { Pane } from "./pane";
import type { Item } from "./types";

/** Remote files bigger than this keep their icon (a thumbnail would download the whole file). */
const REMOTE_IMAGE_MAX = 25 * 1024 * 1024;
/** Presigned links are cached per object for a while, so scrolling doesn't re-sign everything. */
const LINK_TTL = 50 * 60 * 1000;
const links = new Map<string, { url: string; at: number }>();
/** Loaded elements kept for reuse. Decoded pictures cost memory: ~256 KB per thumbnail, and a
 * video frame is full size, hence the small video count. */
const KEEP = { image: 120, video: 8 } as const;
const kept = { image: new Map<string, HTMLElement>(), video: new Map<string, HTMLElement>() };
const prefetched = new Set<string>();
const NXR = navigator.userAgent.includes("Windows") ? "http://nxr.localhost" : "nxr://localhost";

async function srcFor(p: Pane, key: string): Promise<string> {
  if (!p.isRemote) return localSrc(key);
  const id = `${p.account}|${p.bucket}|${key}`;
  const hit = links.get(id);
  if (hit && Date.now() - hit.at < LINK_TTL) return hit.url;
  const url = await invoke<string>("presign", { ...p.where(), key, secs: 3600, inline: true });
  links.set(id, { url, at: Date.now() });
  if (links.size > 2000) links.delete(links.keys().next().value!);
  return url;
}

/** The backend thumbnail of an image; null when there's none to ask for. Size and modified time
 * are in the URL: an edited file gets a new thumbnail. */
function thumbUrl(p: Pane, e: Item): string | null {
  if (p.isRemote && (!p.account || e.size > REMOTE_IMAGE_MAX)) return null;
  const [acct, container] = p.isRemote ? [p.account, p.bucket] : ["~local", "f"];
  const q = encodeURIComponent;
  return `${NXR}/~thumb/${q(acct)}/${q(container)}/${q(e.key)}?${e.size}-${e.modified ?? 0}`;
}

const idOf = (p: Pane, e: Item) =>
  `${p.isRemote ? p.account : "~local"}|${p.bucket}|${e.key}|${e.size}|${e.modified ?? ""}`;

function keep(kind: "image" | "video", id: string, el: HTMLElement) {
  const m = kept[kind];
  m.delete(id);
  m.set(id, el);
  while (m.size > KEEP[kind]) m.delete(m.keys().next().value!);
}

function media(kind: "image" | "video", url: string): HTMLImageElement | HTMLVideoElement {
  const el =
    kind === "image"
      ? Object.assign(document.createElement("img"), { decoding: "async", alt: "", src: url })
      : Object.assign(document.createElement("video"), {
          muted: true,
          preload: "metadata",
          // The frame one second in (a media fragment): no download beyond what's needed.
          src: `${url}${url.includes("#") ? "" : "#t=1"}`,
        });
  el.className = "thumb";
  return el;
}

/** Fills the drawn tiles' thumbnails (called after every grid draw). */
export function fillThumbs(p: Pane) {
  for (const el of p.list.querySelectorAll<HTMLElement>(".cell[data-thumb]")) {
    const e = p.view[+el.dataset.i!];
    const tile = el.querySelector<HTMLElement>(".tile");
    if (!e || !tile) continue;
    const kind = el.dataset.thumb as "image" | "video";
    const id = idOf(p, e);
    const old = kept[kind].get(id);
    if (old) {
      // Already loaded: move it in, no fade (the picture was on screen a moment ago).
      keep(kind, id, old);
      tile.classList.add("has-thumb", "instant");
      tile.append(old);
      continue;
    }
    if (p.isRemote && e.size > REMOTE_IMAGE_MAX) continue;
    const show = (url: string, fallback?: () => void) => {
      if (!el.isConnected || p.view[+el.dataset.i!]?.key !== e.key) return;
      const m = media(kind, url);
      // Keep the icon if the file can't be shown (unsupported format, link refused…).
      m.addEventListener("error", () => (m.remove(), fallback?.()), { once: true });
      m.addEventListener(
        kind === "image" ? "load" : "loadeddata",
        () => (tile.classList.add("has-thumb"), keep(kind, id, m)),
        { once: true },
      );
      tile.append(m);
    };
    const original = () =>
      srcFor(p, e.key).then(
        (url) => show(url),
        () => {},
      );
    const thumb = kind === "image" ? thumbUrl(p, e) : null;
    // A format the backend can't shrink (HEIC, TIFF…) may still show from the file itself.
    if (thumb) show(thumb, original);
    else original();
  }
  prefetch(p);
}

/** Requests the thumbnails of the screen after the drawn tiles, at low priority. */
function prefetch(p: Pane) {
  const cells = p.list.querySelectorAll<HTMLElement>(".cell");
  if (!cells.length) return;
  const last = +cells[cells.length - 1].dataset.i!;
  for (let i = last + 1; i <= last + cells.length && i < p.view.length; i++) {
    const e = p.view[i];
    if (e.dir || kindOf(e.name) !== "image") continue;
    const url = thumbUrl(p, e);
    if (!url || prefetched.has(url)) continue;
    prefetched.add(url);
    if (prefetched.size > 3000) prefetched.delete(prefetched.values().next().value!);
    Object.assign(new Image(), { fetchPriority: "low", decoding: "async", src: url });
  }
}
