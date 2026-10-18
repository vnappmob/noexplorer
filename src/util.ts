// DOM helpers, formatting and small UI primitives (toast, text prompt). No app state here.

export const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

/** localStorage wrapper that never throws (private windows, blocked storage). */
export const store = {
  get: (k: string) => {
    try {
      return localStorage.getItem(k);
    } catch {
      return null;
    }
  },
  set: (k: string, v: string) => {
    try {
      localStorage.setItem(k, v);
    } catch {
      /* ignore */
    }
  },
};

export const esc = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

export const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

export function fmtSize(n: number) {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let i = -1;
  do {
    n /= 1024;
    i++;
  } while (n >= 1024 && i < u.length - 1);
  return `${n.toFixed(n < 10 ? 1 : 0)} ${u[i]}`;
}

const dateFmt = new Intl.DateTimeFormat(undefined, { dateStyle: "short", timeStyle: "short" });
export const fmtDate = (ms: number | null) => (ms ? dateFmt.format(new Date(ms)) : "");

let toastTimer = 0;
export function toast(msg: string, isErr = false) {
  const t = $("toast");
  t.textContent = msg;
  t.className = isErr ? "err" : "";
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (t.hidden = true), isErr ? 6000 : 2200);
}
export const fail = (e: unknown) => toast(String(e), true);

/** Text prompt using the #ask dialog (window.prompt isn't available in every webview). */
export function askText(title: string, value = "", selectStem = false): Promise<string | null> {
  const d = $<HTMLDialogElement>("ask");
  const input = $<HTMLInputElement>("ask-input");
  $("ask-title").textContent = title;
  input.value = value;
  d.returnValue = "";
  $("ask-cancel").onclick = () => d.close("cancel");
  d.showModal();
  input.focus();
  // Renaming: select the name without its extension, like Finder.
  const dot = value.lastIndexOf(".");
  if (selectStem) input.setSelectionRange(0, dot > 0 ? dot : value.length);
  return new Promise((resolve) => {
    d.addEventListener("close", () => resolve(d.returnValue === "ok" ? input.value : null), { once: true });
  });
}
