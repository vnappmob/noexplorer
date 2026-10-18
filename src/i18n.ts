// Vietnamese interface (experimental "vietnamese"). The UI is written in English; this swaps the
// shown text for Vietnamese: the page at boot, then anything drawn later (a MutationObserver).
// Exact phrases come from `VI`, text with numbers or names from `VI_RULES` (i18n-vi.ts).
//
// User content is never touched: file rows, path crumbs, account and bucket names, transfer labels.
// Turning it on or off reloads the window.

import { exp } from "./state";
import { VI, VI_RULES } from "./i18n-vi";

/** Where user content lives (file and folder names, paths, account names). */
const SKIP =
  ".rows, .crumbs, .path-input, .ptab-name, .lane-path, .cname, .act-name, #jobs-list .label, .profile-label, .bucket-label, .chip-sub, #acct-items, #places-sub, .pv-cap, #pv-title, input, textarea";
const ATTRS = ["title", "placeholder", "aria-label"] as const;

function tr(s: string): string | null {
  const t = s.trim();
  if (!t) return null;
  const hit = VI[t];
  if (hit !== undefined) return s.replace(t, hit);
  for (const [re, to] of VI_RULES) if (re.test(t)) return s.replace(t, t.replace(re, to));
  return null;
}

function translateTree(root: Node) {
  const el = root.nodeType === Node.ELEMENT_NODE ? (root as Element) : root.parentElement;
  if (!el || el.closest(SKIP)) return;
  if (root.nodeType === Node.TEXT_NODE) {
    const v = tr(root.nodeValue ?? "");
    if (v !== null) root.nodeValue = v;
    return;
  }
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: (n) =>
      n.nodeType === Node.ELEMENT_NODE && (n as Element).matches(SKIP)
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
  });
  for (let n: Node | null = root; n; n = walker.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) {
      const v = tr(n.nodeValue ?? "");
      if (v !== null) n.nodeValue = v;
    } else {
      const e = n as Element;
      for (const a of ATTRS) {
        const cur = e.getAttribute(a);
        const v = cur && tr(cur);
        if (v) e.setAttribute(a, v);
      }
    }
  }
}

export function setupI18n() {
  // Reload so every part of the UI starts in the new language.
  let on = false;
  window.addEventListener("settings-loaded", () => {
    on = exp("vietnamese");
    if (!on) return;
    document.documentElement.lang = "vi";
    translateTree(document.body);
    new MutationObserver((list) => {
      for (const m of list) {
        if (m.type === "characterData") translateTree(m.target);
        else m.addedNodes.forEach((n) => translateTree(n));
        if (m.type === "attributes" && m.target instanceof Element) translateTree(m.target);
      }
    }).observe(document.body, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: [...ATTRS],
    });
  });
  window.addEventListener("experimental", () => {
    if (exp("vietnamese") !== on) location.reload();
  });
}
