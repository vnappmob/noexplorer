// One file list (local or remote): navigation, listing, sorting, filtering, selection, rendering.
// A Pane knows nothing about transfers/preview; it calls `paneHooks` for those (wired in main.ts).

import { invoke } from "@tauri-apps/api/core";
import { kindOf } from "./files";
import { icon } from "./icons";
import { activeProfile, app, profileKey } from "./state";
import type { Crumb, Item, LocalListing, SortBy } from "./types";
import { $, esc, fail, fmtDate, fmtSize, plural, store } from "./util";

/** Row icon per file kind (files.ts `kindOf`). */
const FILE_ICON: Record<string, string> = {
  image: "image",
  video: "film",
  audio: "music",
  pdf: "doc",
  text: "doc",
  archive: "archive",
  other: "doc",
};

/** Rows kept in the DOM beyond the visible ones, on each side (see `drawRows`). */
const OVERSCAN = 15;

export const paneHooks = {
  /** Enter / double-click on a file. */
  openFile: (_p: Pane, _e: Item): void => {},
};

export class Pane {
  readonly el: HTMLElement;
  readonly list: HTMLElement;
  /** The opposite pane (set by panes.ts). */
  other!: Pane;
  /** Remote: prefix ("" = bucket root, else ends with "/"). Local: absolute folder path. */
  cwd = "";
  /** Local only: parent folder, null at a filesystem root. */
  parent: string | null = null;
  crumbs: Crumb[] = [];
  /** Local only: Home, common folders, disks and volumes (the "This computer" menu). */
  roots: Crumb[] = [];
  /** Everything in `cwd`. */
  entries: Item[] = [];
  /** `entries` after filter + sort — what's on screen; row `data-i` indexes into this. */
  view: Item[] = [];
  selected = new Set<string>();
  /** Anchor row for shift-click / shift-arrow range selection. */
  anchor = -1;
  sortBy: SortBy;
  sortDir: number;
  /** Guards against out-of-order async listings. */
  private seq = 0;
  /** Virtualized list: only rows near the viewport are in the DOM, inside `.rows`, which is as tall
   * as all of them (a width change, like dragging the splitter, would otherwise re-lay out every row
   * of a big folder). `rowH` is measured from a rendered row; `win` = [first, end) rows in the DOM. */
  private rowsEl: HTMLElement | null = null;
  private rowH = 0;
  private win: [number, number] = [0, 0];

  constructor(readonly kind: "local" | "remote") {
    this.el = $(`pane-${kind}`);
    this.list = this.q(".list");
    this.sortBy = (store.get(`${kind}.sortBy`) as SortBy) || "name";
    this.sortDir = store.get(`${kind}.sortDir`) === "desc" ? -1 : 1;
    // Watermark: a fixed background behind the rows (the list is transparent and sits above it),
    // so it stays visible however long the list is or wherever it's scrolled.
    const mark = this.isRemote ? "markCloud" : "markComputer";
    this.el.insertAdjacentHTML(
      "beforeend",
      `<div class="mark mark-${kind}" aria-hidden="true"><span>${icon(mark)}</span></div>`,
    );
    // Called straight from the events (no rAF: it pauses in covered windows); drawRows returns
    // early while the visible rows are already in the DOM.
    const redraw = () => this.drawRows();
    this.list.addEventListener("scroll", redraw, { passive: true });
    new ResizeObserver(redraw).observe(this.list);
    // Text size changes the row height.
    window.addEventListener("textsize", () => ((this.rowH = 0), this.drawRows(true)));
  }

  q<T extends HTMLElement = HTMLElement>(sel: string) {
    return this.el.querySelector(sel) as T;
  }

  get isRemote() {
    return this.kind === "remote";
  }

  /** Re-lists `cwd`. `keepSelection` keeps still-existing selected rows (used after refreshes). */
  async load(keepSelection = false): Promise<void> {
    const seq = ++this.seq;
    try {
      if (this.isRemote) await this.loadRemote(seq);
      else await this.loadLocal(seq);
      if (seq !== this.seq) return;
    } catch (e) {
      if (seq !== this.seq) return;
      this.entries = [];
      if (this.cwd) {
        // Folder may have vanished; fall back to the parent / root.
        this.cwd = this.isRemote ? "" : (this.parent ?? "");
        return this.load();
      }
      fail(e);
    }
    if (!keepSelection) {
      this.selected.clear();
      this.anchor = -1;
    } else {
      const keys = new Set(this.entries.map((e) => e.key));
      this.selected = new Set([...this.selected].filter((k) => keys.has(k)));
    }
    this.render();
  }

  private async loadRemote(seq: number) {
    store.set(profileKey("bucket"), app.bucket);
    store.set(profileKey("prefix"), this.cwd);
    this.crumbs = [{ name: app.bucket || "—", path: "" }];
    let acc = "";
    for (const p of this.cwd.split("/").filter(Boolean)) {
      acc += p + "/";
      this.crumbs.push({ name: p, path: acc });
    }
    this.renderCrumbs();
    if (!app.bucket) {
      this.entries = [];
      return;
    }
    this.q(".count").textContent = "· loading…";
    const list = await invoke<Item[]>("list", { bucket: app.bucket, prefix: this.cwd });
    if (seq === this.seq) this.entries = list;
  }

  private async loadLocal(seq: number) {
    const l = await invoke<LocalListing>("list_local", { path: this.cwd || null, hidden: app.showHidden });
    if (seq !== this.seq) return;
    this.cwd = l.path;
    this.parent = l.parent;
    this.crumbs = l.crumbs;
    this.entries = l.entries;
    store.set("localPath", this.cwd);
    this.roots = l.roots;
    // Chip subtitle: the place we're in (longest matching root), e.g. "Home", "Downloads", "T7 Shield".
    const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
    const here = norm(this.cwd);
    const place = l.roots
      .filter((r) => here === norm(r.path) || here.startsWith(norm(r.path) + "/"))
      .sort((a, b) => b.path.length - a.path.length)[0];
    // Root names come with an emoji prefix from the backend ("🏠 Home"); the chip has its own icon.
    $("places-sub").textContent = place ? place.name.replace(/^\p{Extended_Pictographic}\s*/u, "") : "";
    this.renderCrumbs();
  }

  /** Full location of `cwd`: an absolute local path, or `s3://bucket/prefix`. */
  fullPath() {
    return this.isRemote ? (app.bucket ? `s3://${app.bucket}/${this.cwd}` : "") : this.cwd;
  }

  go(path: string, keepSelection = false) {
    this.cwd = path;
    this.q<HTMLInputElement>(".filter").value = "";
    this.list.scrollTop = 0;
    return this.load(keepSelection);
  }

  /** Shows/hides the filter box in the header (⌘F, the magnifier button); hiding clears it. */
  toggleFilter(on: boolean) {
    const f = this.q<HTMLInputElement>(".filter");
    f.hidden = !on;
    this.q(".filter-btn").classList.toggle("on", on);
    if (on) f.focus();
    else if (f.value) {
      f.value = "";
      this.render();
    }
  }

  /** Goes to the parent folder and re-selects the folder we came out of. */
  up() {
    if (this.isRemote) {
      if (!this.cwd) return;
      const parts = this.cwd.split("/").filter(Boolean);
      this.selected = new Set([this.cwd]);
      parts.pop();
      this.go(parts.length ? parts.join("/") + "/" : "", true);
    } else if (this.parent) {
      this.selected = new Set([this.cwd]);
      this.go(this.parent, true);
    }
  }

  /** Path bar row: one button per folder level (click = go there, drop target while dragging). */
  renderCrumbs() {
    const el = this.q(".crumbs");
    const sep = `<span class="crumb-sep">${icon("chevRight")}</span>`;
    el.innerHTML = this.crumbs
      .map((c, i) => {
        const last = i === this.crumbs.length - 1 ? " last" : "";
        // The filesystem root ("/" or "C:\\") reads better as a disk icon.
        const label =
          !this.isRemote && i === 0 && /^([/\\]|[A-Za-z]:[\\/]?)$/.test(c.name) ? icon("drive") : esc(c.name);
        return `${i ? sep : ""}<button class="crumb${last}" data-p="${esc(c.path)}" title="${esc(c.path || c.name)}">${label}</button>`;
      })
      .join("");
    el.scrollLeft = el.scrollWidth; // keep the deepest folder visible
    this.q<HTMLButtonElement>(".up").disabled = this.isRemote ? !this.cwd : !this.parent;
    this.q(".dirsize").textContent = "Size"; // folder size resets per folder
    this.q(".calc-size").classList.remove("has");
  }

  render() {
    const q = this.q<HTMLInputElement>(".filter").value.trim().toLowerCase();
    this.view = this.entries.filter((e) => !q || e.name.toLowerCase().includes(q));
    const { sortBy, sortDir } = this;
    this.view.sort((a, b) => {
      if (a.dir !== b.dir) return a.dir ? -1 : 1; // folders first
      let r = 0;
      if (sortBy === "size") r = a.size - b.size;
      else if (sortBy === "modified") r = (a.modified ?? 0) - (b.modified ?? 0);
      if (r === 0) r = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
      return r * sortDir;
    });
    this.el.querySelectorAll<HTMLElement>(".cols span").forEach((s) => {
      s.className = s.dataset.sort === sortBy ? (sortDir > 0 ? "asc" : "desc") : "";
    });

    this.rowsEl = null;
    if (this.isRemote && !activeProfile()) {
      this.list.innerHTML = `<div class="empty"><b>No S3 account yet</b>Click the account button above to add one.</div>`;
    } else if (!this.view.length) {
      const hint = this.isRemote
        ? "Drag files here from the left pane or from Finder / Explorer to upload."
        : "Drag files here from the bucket to download.";
      this.list.innerHTML = `<div class="empty">${q ? "<b>No matches</b>" : `<b>Empty folder</b>${hint}`}</div>`;
    } else {
      this.list.innerHTML = `<div class="rows"></div>`;
      this.rowsEl = this.list.firstElementChild as HTMLElement;
      this.drawRows(true);
    }
    this.updateStatus();
  }

  private rowHtml(i: number) {
    const e = this.view[i];
    return `<div class="row${this.selected.has(e.key) ? " sel" : ""}" data-i="${i}"${e.dir ? ` data-dir="${esc(e.key)}"` : ""}>
      <span class="name"><span class="ico${e.dir ? " dir" : ""}">${icon(e.dir ? "folder" : FILE_ICON[kindOf(e.name)])}</span><span>${esc(e.name)}</span></span>
      <span class="size${e.calc ? " calc" : ""}">${e.dir && !e.calc ? "" : fmtSize(e.size)}</span>
      <span class="date">${fmtDate(e.modified)}</span>
    </div>`;
  }

  /** Puts the rows around the viewport in the DOM. Skipped while the visible rows are already
   * there, so scrolling rebuilds the window only every ~OVERSCAN rows. `force` = rows changed. */
  private drawRows(force = false) {
    const box = this.rowsEl;
    if (!box) return;
    const n = this.view.length;
    if (!this.rowH) {
      box.innerHTML = this.rowHtml(0);
      this.rowH = box.firstElementChild!.getBoundingClientRect().height || 36;
      force = true;
    }
    const h = this.rowH;
    const top = this.list.scrollTop - box.offsetTop;
    const first = Math.max(0, Math.floor(top / h));
    const end = Math.min(n, Math.ceil((top + this.list.clientHeight) / h));
    if (!force && first >= this.win[0] && end <= this.win[1]) return;
    const a = Math.max(0, first - OVERSCAN);
    const b = Math.min(n, end + OVERSCAN);
    this.win = [a, b];
    box.style.height = `${n * h}px`;
    box.style.paddingTop = `${a * h}px`;
    let html = "";
    for (let i = a; i < b; i++) html += this.rowHtml(i);
    box.innerHTML = html;
  }

  /** Scrolls row `i` into view (it may not be in the DOM yet). */
  private reveal(i: number) {
    if (!this.rowsEl || !this.rowH) return;
    const l = this.list;
    const top = this.rowsEl.offsetTop + i * this.rowH;
    if (top < l.scrollTop) l.scrollTop = top;
    else if (top + this.rowH > l.scrollTop + l.clientHeight) l.scrollTop = top + this.rowH - l.clientHeight;
    this.drawRows();
  }

  selectedItems() {
    return this.view.filter((e) => this.selected.has(e.key));
  }

  /** Column-header summary: item count and size, or the selection's count and size. */
  updateStatus() {
    const sel = this.selectedItems();
    const n = this.view.length;
    if (sel.length) {
      const sz = sel.reduce((s, e) => s + e.size, 0);
      this.q(".count").textContent = `· ${sel.length} of ${n} selected${sz ? ` · ${fmtSize(sz)}` : ""}`;
    } else {
      const total = this.view.reduce((s, e) => s + e.size, 0);
      this.q(".count").textContent = `· ${plural(n, "item")}${total ? ` · ${fmtSize(total)}` : ""}`;
    }
  }

  /** Repaints selection highlight without re-rendering rows. */
  paint() {
    this.list.querySelectorAll<HTMLElement>(".row").forEach((r) => {
      r.classList.toggle("sel", this.selected.has(this.view[+r.dataset.i!].key));
    });
    this.updateStatus();
  }

  selectIndex(i: number, extend = false) {
    if (!this.view.length) return;
    i = Math.max(0, Math.min(this.view.length - 1, i));
    if (extend && this.anchor >= 0) {
      const [a, b] = [Math.min(this.anchor, i), Math.max(this.anchor, i)];
      this.selected = new Set(this.view.slice(a, b + 1).map((e) => e.key));
    } else {
      this.selected = new Set([this.view[i].key]);
      this.anchor = i;
    }
    this.paint();
    this.reveal(i);
  }

  /** Index of the first selected row in `view`, or -1. */
  cursor() {
    return this.view.findIndex((e) => this.selected.has(e.key));
  }

  /** Enter / double-click: open folders here, hand files to the app. */
  activate(e: Item) {
    if (e.dir) this.go(e.key);
    else paneHooks.openFile(this, e);
  }
}
