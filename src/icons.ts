// Stroke icons (24px grid, currentColor) used by the markup (`<span data-icon="name">`, filled in
// by `hydrateIcons`) and by menus/rows built in code (`icon("name")`). Keep them simple: one
// look for the whole app (see docs/DECISIONS.md ADR-019).

const svg = (body: string, extra = "") =>
  `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"${extra}>${body}</svg>`;

export const ICONS: Record<string, string> = {
  search: svg('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>'),
  folderPlus: svg(
    '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M12 11v5M9.5 13.5h5"/>',
  ),
  refresh: svg('<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/>'),
  more: `<svg class="i" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/></svg>`,
  chevDown: svg('<path d="m6 9 6 6 6-6"/>'),
  chevRight: svg('<path d="m9 6 6 6-6 6"/>'),
  arrowUp: svg('<path d="M12 19V5M6 11l6-6 6 6"/>'),
  arrowRight: svg('<path d="M5 12h14M13 6l6 6-6 6"/>'),
  arrowLeft: svg('<path d="M19 12H5M11 6l-6 6 6 6"/>'),
  arrowDown: svg('<path d="M12 5v14M6 13l6 6 6-6"/>'),
  copy: svg('<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a2 2 0 0 1 2-2h9"/>'),
  sigma: svg('<path d="M18 5H6l6 7-6 7h12"/>'),
  compare: svg('<path d="M4 8h13l-3-3M20 16H7l3 3"/>'),
  split: svg('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 12h18"/>'),
  sliders: svg(
    '<path d="M4 7h10M18 7h2M4 17h4M12 17h8"/><circle cx="16" cy="7" r="2"/><circle cx="10" cy="17" r="2"/>',
  ),
  eye: svg('<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>'),
  external: svg('<path d="M14 4h6v6M20 4l-9 9"/><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"/>'),
  link: svg(
    '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
  ),
  trash: svg('<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/>'),
  check: svg('<path d="m5 12 5 5L20 7"/>'),
  x: svg('<path d="M6 6l12 12M18 6 6 18"/>'),
  plus: svg('<path d="M12 5v14M5 12h14"/>'),
  home: svg('<path d="M4 11 12 4l8 7"/><path d="M6 10v9h12v-9"/>'),
  drive: svg('<rect x="3" y="13" width="18" height="7" rx="2"/><path d="M5 13 7.5 5h9L19 13"/><path d="M7 16.5h.01"/>'),
  desktop: svg('<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>'),
  download: svg('<path d="M12 4v10M8 10l4 4 4-4M5 20h14"/>'),
  doc: svg('<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>'),
  pause: svg('<path d="M9 5v14M15 5v14"/>'),
  play: svg('<path d="M8 5l11 7-11 7z"/>'),
  cloud: svg('<path d="M7 18a5 5 0 1 1 .9-9.9A6 6 0 0 1 19 10a4 4 0 0 1-1 8z"/>'),
  bucket: svg('<path d="M4 7h16l-2 12a2 2 0 0 1-2 1.7H8A2 2 0 0 1 6 19z"/><ellipse cx="12" cy="7" rx="8" ry="2.5"/>'),
  grid: svg('<path d="M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z"/>'),
  list: svg('<path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01"/>'),
  star: svg('<path d="M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 16.9 6.8 19.6l1-5.8-4.3-4.1 5.9-.9z"/>'),
  folder: svg('<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>'),
  image: svg(
    '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="10" r="2"/><path d="m21 16-5-5L5 20"/>',
  ),
  film: svg('<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m10 9 5 3-5 3z"/>'),
  music: svg('<path d="M9 18V6l11-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/>'),
  archive: svg(
    '<rect x="3" y="4" width="18" height="5" rx="1"/><path d="M5 9v10a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V9M10 13h4"/>',
  ),
  pencil: svg('<path d="M4 20h4L19 9l-4-4L4 16z"/>'),
  update: svg('<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/><path d="M12 8v5l3 2"/>'),
  sun: svg(
    '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
  ),
  // Large faint watermarks that tell the panes apart at a glance (styles.css .mark).
  markComputer: `<svg class="mark-svg" viewBox="0 0 230 200" fill="none" aria-hidden="true"><rect x="22" y="14" width="186" height="124" rx="14" fill="currentColor" fill-opacity=".05" stroke="currentColor" stroke-opacity=".09" stroke-width="3"/><rect x="38" y="30" width="154" height="92" rx="6" fill="currentColor" fill-opacity=".03"/><path d="M96 138v26M134 138v26" stroke="currentColor" stroke-opacity=".09" stroke-width="3"/><rect x="72" y="164" width="86" height="10" rx="5" fill="currentColor" fill-opacity=".07"/></svg>`,
  markCloud: `<svg class="mark-svg" viewBox="0 0 24 16" fill="none" aria-hidden="true"><path d="M6.5 15a5 5 0 1 1 .9-9.9A6 6 0 0 1 18.5 7a4 4 0 0 1-1 8z" fill="currentColor" fill-opacity=".08" stroke="currentColor" stroke-opacity=".12" stroke-width=".22"/></svg><svg class="mark-svg mark-mini" viewBox="0 0 24 16" fill="none" aria-hidden="true"><path d="M6.5 15a5 5 0 1 1 .9-9.9A6 6 0 0 1 18.5 7a4 4 0 0 1-1 8z" fill="currentColor" fill-opacity=".06"/></svg>`,
};

export const icon = (name: string) => ICONS[name] ?? "";

/** Fills every `<… data-icon="name">` in `root` with its SVG (idempotent). */
export function hydrateIcons(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("[data-icon]").forEach((el) => {
    if (!el.firstElementChild) el.innerHTML = icon(el.dataset.icon!);
  });
}
