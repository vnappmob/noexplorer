// Grid view thumbnails (experimental "grid"). Tiles of images and videos show the file itself:
// local files through the asset protocol, remote ones through the same short-lived link quick
// look uses. The browser does the decoding (no canvas: remote links usually have no CORS), and
// only tiles near the viewport exist (the grid is virtualized), so only those load.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { Pane } from "./pane";

/** Remote images bigger than this keep their icon (a thumbnail would download the whole file). */
const REMOTE_IMAGE_MAX = 25 * 1024 * 1024;
/** Presigned links are cached per object for a while, so scrolling doesn't re-sign everything. */
const LINK_TTL = 50 * 60 * 1000;
const links = new Map<string, { url: string; at: number }>();

async function srcFor(p: Pane, key: string): Promise<string> {
  if (!p.isRemote) return convertFileSrc(key);
  const id = `${p.account}|${p.bucket}|${key}`;
  const hit = links.get(id);
  if (hit && Date.now() - hit.at < LINK_TTL) return hit.url;
  const url = await invoke<string>("presign", { ...p.where(), key, secs: 3600, inline: true });
  links.set(id, { url, at: Date.now() });
  if (links.size > 2000) links.delete(links.keys().next().value!);
  return url;
}

/** Fills the drawn tiles' thumbnails (called after every grid draw). */
export function fillThumbs(p: Pane) {
  for (const el of p.list.querySelectorAll<HTMLElement>(".cell[data-thumb]")) {
    const e = p.view[+el.dataset.i!];
    if (!e) continue;
    const kind = el.dataset.thumb!;
    if (p.isRemote && kind === "image" && e.size > REMOTE_IMAGE_MAX) continue;
    srcFor(p, e.key).then(
      (url) => {
        const tile = el.querySelector<HTMLElement>(".tile");
        if (!tile || !el.isConnected || p.view[+el.dataset.i!]?.key !== e.key) return;
        const media =
          kind === "image"
            ? Object.assign(document.createElement("img"), { loading: "lazy", decoding: "async", alt: "", src: url })
            : Object.assign(document.createElement("video"), {
                muted: true,
                preload: "metadata",
                // The frame one second in (a media fragment): no download beyond what's needed.
                src: `${url}${url.includes("#") ? "" : "#t=1"}`,
              });
        media.className = "thumb";
        // Keep the icon if the file can't be shown (unsupported format, link refused…).
        media.addEventListener("error", () => media.remove());
        media.addEventListener(kind === "image" ? "load" : "loadeddata", () => tile.classList.add("has-thumb"));
        tile.append(media);
      },
      () => {},
    );
  }
}
