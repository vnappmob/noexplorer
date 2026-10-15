// One file list (local or remote): navigation, listing, sorting, filtering, selection, rendering.
// A Pane knows nothing about transfers/preview; it calls `paneHooks` for those (wired in main.ts).

import { invoke } from "@tauri-apps/api/core";
import { ICON, kindOf } from "./files";
import { app } from "./state";
import type { Crumb, Item, LocalListing, SortBy } from "./types";
import { $, esc, fail, fmtDate, fmtSize, plural, store } from "./util";

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

  constructor(readonly kind: "local" | "remote") {
    this.el = $(`pane-${kind}`);
    this.list = this.q(".list");
    this.sortBy = (store.get(`${kind}.sortBy`) as SortBy) || "name";
    this.sortDir = store.get(`${kind}.sortDir`) === "desc" ? -1 : 1;
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
    store.set("bucket", app.bucket);
    store.set("prefix", this.cwd);
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
    this.q(".status-text").textContent = "Loading…";
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
    this.q(".roots").innerHTML =
      `<option value="" disabled selected>📍 Places</option>` +
      l.roots.map((r) => `<option value="${esc(r.path)}">${esc(r.name)}</option>`).join("");
    this.renderCrumbs();
  }

  go(path: string, keepSelection = false) {
    this.cwd = path;
    this.q<HTMLInputElement>(".filter").value = "";
    this.list.scrollTop = 0;
    return this.load(keepSelection);
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

  renderCrumbs() {
    const el = this.q(".crumbs");
    el.innerHTML = this.crumbs
      .map((c, i) => `${i ? '<span class="sep">›</span>' : ""}<a data-p="${esc(c.path)}">${esc(c.name)}</a>`)
      .join("");
    el.scrollLeft = el.scrollWidth; // keep the deepest folder visible
    this.q<HTMLButtonElement>(".up").disabled = this.isRemote ? !this.cwd : !this.parent;
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

    if (this.isRemote && !app.cfg) {
      this.list.innerHTML = `<div class="empty"><div class="big">⚙</div>Not connected.<br/>Click ⚙ to set up your S3 server.</div>`;
    } else if (!this.view.length) {
      const hint = this.isRemote
        ? "Drag files here from the left pane<br/>or from Finder / Explorer to upload."
        : "Drag files here from S3 (right pane) to download.";
      this.list.innerHTML = `<div class="empty"><div class="big">${this.isRemote ? "⇪" : "📂"}</div>${
        q ? "No matches." : `Empty folder.<br/>${hint}`
      }</div>`;
    } else {
      this.list.innerHTML = this.view
        .map(
          (e, i) => `<div class="row${this.selected.has(e.key) ? " sel" : ""}" data-i="${i}"${
            e.dir ? ` data-dir="${esc(e.key)}"` : ""
          }>
          <span class="name"><span class="ico">${e.dir ? "📁" : ICON[kindOf(e.name)]}</span><span>${esc(e.name)}</span></span>
          <span class="size">${e.dir ? "" : fmtSize(e.size)}</span>
          <span class="date">${fmtDate(e.modified)}</span>
        </div>`,
        )
        .join("");
    }
    this.updateStatus();
  }

  selectedItems() {
    return this.view.filter((e) => this.selected.has(e.key));
  }

  /** Footer text + enabled state of the footer buttons (`[data-act]`). */
  updateStatus() {
    const sel = this.selectedItems();
    const total = this.view.reduce((s, e) => s + e.size, 0);
    let txt = `${plural(this.view.length, "item")} · ${fmtSize(total)}`;
    if (sel.length) {
      const sz = sel.reduce((s, e) => s + e.size, 0);
      txt = `${sel.length} of ${this.view.length} selected${sz ? ` · ${fmtSize(sz)}` : ""}`;
    }
    this.q(".status-text").textContent = txt;
    const oneFile = sel.length === 1 && !sel[0].dir;
    const enable: Record<string, boolean> = {
      preview: oneFile,
      open: oneFile,
      link: oneFile,
      transfer: sel.length > 0 && !!app.bucket,
      delete: sel.length > 0,
    };
    this.el.querySelectorAll<HTMLButtonElement>("[data-act]").forEach((b) => {
      b.disabled = !(enable[b.dataset.act!] ?? true);
    });
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
    this.list.querySelector(`.row[data-i="${i}"]`)?.scrollIntoView({ block: "nearest" });
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
