// Builds the docs site: site-src/docs/<slug>.html (page bodies) + _layout.html → site/docs/<slug>.html.
// The website has no build step on Cloudflare, so the output is committed. `--check` fails when it's
// stale (npm run check runs it, so CI catches a forgotten rebuild).
//
// Usage: node scripts/build-docs.mjs [--check]

import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { format } from "prettier";

/** Sidebar order. `slug: "index"` is /docs/. */
const PAGES = [
  { slug: "index", title: "Getting started", desc: "Install NoExplorer and move your first files." },
  { slug: "accounts", title: "Connect your storage", desc: "Add S3, Nextcloud, WebDAV or Box accounts." },
  { slug: "transfers", title: "Moving files", desc: "Upload, download and copy between accounts." },
  { slug: "tools", title: "Compare, sizes & more", desc: "Compare folders, folder sizes, quick look, links." },
  { slug: "shortcuts", title: "Keyboard shortcuts", desc: "Every shortcut in one table." },
  { slug: "faq", title: "FAQ & troubleshooting", desc: "Answers to common questions and problems." },
  { slug: "privacy", title: "Privacy", desc: "What NoExplorer sends, and where." },
  {
    slug: "code-signing",
    title: "Code signing",
    desc: "How downloads and updates are signed, and installing on Windows.",
  },
];

const SRC = "site-src/docs";
const OUT = "site/docs";
const check = process.argv.includes("--check");
const href = (slug) => (slug === "index" ? "/docs/" : `/docs/${slug}`);
const layout = readFileSync(`${SRC}/_layout.html`, "utf8");

let stale = [];
mkdirSync(OUT, { recursive: true });
for (const [i, page] of PAGES.entries()) {
  const nav = PAGES.map(
    (p) => `<a href="${href(p.slug)}"${p === page ? ' aria-current="page"' : ""}>${p.title}</a>`,
  ).join("\n");
  const prev = PAGES[i - 1];
  const next = PAGES[i + 1];
  const pager = [
    prev ? `<a class="prev" href="${href(prev.slug)}">← ${prev.title}</a>` : "<span></span>",
    next ? `<a class="next" href="${href(next.slug)}">${next.title} →</a>` : "<span></span>",
  ].join("\n");
  const html = layout
    .replaceAll("{{title}}", page.title)
    .replaceAll("{{desc}}", page.desc)
    .replace("{{nav}}", nav)
    .replace("{{pager}}", pager)
    .replace("{{body}}", readFileSync(`${SRC}/${page.slug}.html`, "utf8"));
  const out = await format(html, { parser: "html", printWidth: 120 });
  const file = `${OUT}/${page.slug}.html`;
  if (check) {
    if (!existsSync(file) || readFileSync(file, "utf8") !== out) stale.push(file);
  } else {
    writeFileSync(file, out);
  }
}
if (stale.length) {
  console.error(`Docs site is out of date: ${stale.join(", ")}\nRun: node scripts/build-docs.mjs`);
  process.exit(1);
}
if (!check) console.log(`Built ${PAGES.length} pages into ${OUT}/`);
