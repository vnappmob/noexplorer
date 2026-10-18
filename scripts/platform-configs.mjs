// Writes src-tauri/tauri.macos.conf.json and tauri.windows.conf.json from tauri.conf.json: the same
// main window, but transparent, so the experimental "native look" (macOS translucency, Windows 11
// Mica) can show through. Linux keeps an opaque window (transparent ones can render black there).
// Tauri merges platform files with JSON Merge Patch, which replaces arrays: hence the full window.
//
// Usage: node scripts/platform-configs.mjs [--check]

import { readFileSync, writeFileSync, existsSync } from "node:fs";

const base = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const windows = base.app.windows.map((w) => ({ ...w, transparent: true }));
const files = {
  "src-tauri/tauri.macos.conf.json": { app: { windows } },
  "src-tauri/tauri.windows.conf.json": { app: { windows } },
};
const check = process.argv.includes("--check");
const stale = [];
for (const [file, cfg] of Object.entries(files)) {
  const text = `${JSON.stringify(cfg, null, 2)}\n`; // plain JSON: Tauri config allows no comments
  if (check) {
    if (!existsSync(file) || readFileSync(file, "utf8") !== text) stale.push(file);
  } else writeFileSync(file, text);
}
if (stale.length) {
  console.error(`Out of date: ${stale.join(", ")}\nRun: node scripts/platform-configs.mjs`);
  process.exit(1);
}
