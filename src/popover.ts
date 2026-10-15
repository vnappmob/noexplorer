// Dropdown menus anchored to a button (account, bucket, this computer, "⋯"). One menu open at a
// time in #popover; closes on outside click, Escape, or after choosing. Keyboard: ↑/↓, Enter, typing
// in the optional search box filters the items.

import { icon } from "./icons";
import { $, esc } from "./util";

export type MenuItem =
  | { sep: true }
  | { header: string }
  | {
      label: string;
      sub?: string;
      /** Icon name from icons.ts, shown in a tinted square. */
      icon?: string;
      /** Extra classes for the icon square (e.g. "acct-ico"). */
      iconClass?: string;
      badge?: string;
      checked?: boolean;
      run: () => unknown;
    };

type Runnable = Extract<MenuItem, { run: () => unknown }>;

let anchorEl: HTMLElement | null = null;
let items: MenuItem[] = [];
let hl = -1;

const pop = () => $("popover");
const isRunnable = (m: MenuItem): m is Runnable => "run" in m;

export function closeMenu() {
  pop().hidden = true;
  anchorEl?.setAttribute("aria-expanded", "false");
  anchorEl = null;
}

export function isMenuOpenFor(el: HTMLElement) {
  return anchorEl === el && !pop().hidden;
}

/** Opens a menu under `anchor`. `search` adds a filter box (for long lists like buckets). */
export function openMenu(anchor: HTMLElement, list: MenuItem[], opts: { search?: string; width?: number } = {}) {
  if (isMenuOpenFor(anchor)) return closeMenu(); // second click toggles it closed
  closeMenu();
  anchorEl = anchor;
  items = list;
  anchor.setAttribute("aria-expanded", "true");
  const p = pop();
  p.style.minWidth = `${Math.max(opts.width ?? 0, anchor.offsetWidth, 220)}px`;
  p.innerHTML =
    (opts.search
      ? `<label class="menu-search">${icon("search")}<input id="pop-q" placeholder="${esc(opts.search)}" spellcheck="false" autocomplete="off" /></label>`
      : "") + `<div id="pop-items"></div>`;
  render("");
  p.hidden = false;
  place(anchor);
  const q = p.querySelector<HTMLInputElement>("#pop-q");
  if (q) {
    q.addEventListener("input", () => render(q.value));
    q.focus();
  }
}

function place(anchor: HTMLElement) {
  const p = pop();
  const r = anchor.getBoundingClientRect();
  const w = p.offsetWidth;
  p.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
  const below = r.bottom + 6;
  p.style.top = `${below + p.offsetHeight > innerHeight - 8 ? Math.max(8, r.top - p.offsetHeight - 6) : below}px`;
}

function render(filter: string) {
  const f = filter.trim().toLowerCase();
  const shown = items.filter((m) => !f || (isRunnable(m) && (m.label + " " + (m.sub ?? "")).toLowerCase().includes(f)));
  hl = shown.findIndex(isRunnable);
  $("pop-items").innerHTML =
    shown
      .map((m, i) => {
        if ("sep" in m) return "<hr/>";
        if ("header" in m) return `<div class="mh">${esc(m.header)}</div>`;
        const ico = m.icon ? `<span class="mi-ico ${m.iconClass ?? ""}">${icon(m.icon)}</span>` : "";
        const sub = m.sub ? `<span class="mi-sub">${esc(m.sub)}</span>` : "";
        const badge = m.badge ? `<span class="mi-badge">${esc(m.badge)}</span>` : "";
        const check = m.checked ? `<span class="mi-check">${icon("check")}</span>` : "";
        return `<div class="mi${m.checked ? " checked" : ""}" role="menuitem" data-i="${items.indexOf(m)}" data-s="${i}">${ico}<span class="mi-text"><span>${esc(m.label)}</span>${sub}</span>${badge}${check}</div>`;
      })
      .join("") || `<div class="mh">No matches</div>`;
  paintHl();
}

function rows() {
  return [...$("pop-items").querySelectorAll<HTMLElement>(".mi")];
}

function paintHl() {
  rows().forEach((r, i) => r.classList.toggle("hl", i === hl));
  rows()[hl]?.scrollIntoView({ block: "nearest" });
}

function choose(el: HTMLElement | undefined) {
  const m = el ? items[+el.dataset.i!] : undefined;
  closeMenu();
  if (m && isRunnable(m)) m.run();
}

export function setupPopover() {
  pop().addEventListener("click", (ev) => choose((ev.target as HTMLElement).closest<HTMLElement>(".mi") ?? undefined));
  document.addEventListener("mousedown", (ev) => {
    const t = ev.target as HTMLElement;
    if (!pop().hidden && !t.closest("#popover") && !(anchorEl && anchorEl.contains(t))) closeMenu();
  });
  // Capture phase so the menu gets keys before the global shortcuts (keyboard.ts).
  document.addEventListener(
    "keydown",
    (ev) => {
      if (pop().hidden) return;
      const n = rows().length;
      if (ev.key === "Escape") closeMenu();
      else if (ev.key === "ArrowDown") hl = n ? (hl + 1) % n : -1;
      else if (ev.key === "ArrowUp") hl = n ? (hl - 1 + n) % n : -1;
      else if (ev.key === "Enter") return (ev.preventDefault(), ev.stopPropagation(), choose(rows()[hl]));
      else return;
      ev.preventDefault();
      ev.stopPropagation();
      paintHl();
    },
    true,
  );
  window.addEventListener("resize", closeMenu);
  window.addEventListener("blur", closeMenu);
}
