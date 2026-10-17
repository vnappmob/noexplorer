// Adds the signed Windows installers to the release's latest.json (release.yml → windows-updater-json).
// Signed Windows builds skip tauri-action (ADR-030), so nothing else writes their updater entries.
//
// Usage: node scripts/updater-json-windows.mjs <tag> <dir with *.sig> <current latest.json or missing>
// Prints the merged latest.json. Keys match what tauri-action writes: windows-x86_64 is the MSI.

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const [tag, sigDir, current] = process.argv.slice(2);
const version = tag.replace(/^v/, "");
const json = existsSync(current)
  ? JSON.parse(readFileSync(current, "utf8"))
  : { version, notes: "See CHANGELOG.md for what's new.", pub_date: new Date().toISOString(), platforms: {} };

const entry = (suffix) => {
  const sig = readdirSync(sigDir).find((f) => f.endsWith(`${suffix}.sig`));
  if (!sig) throw new Error(`no signature for *${suffix} in ${sigDir}`);
  const file = sig.slice(0, -".sig".length);
  return {
    signature: readFileSync(join(sigDir, sig), "utf8").trim(),
    url: `https://github.com/vnappmob/noexplorer/releases/download/${tag}/${file}`,
  };
};

const msi = entry(".msi");
json.platforms["windows-x86_64"] = msi;
json.platforms["windows-x86_64-msi"] = msi;
json.platforms["windows-x86_64-nsis"] = entry("-setup.exe");
process.stdout.write(JSON.stringify(json, null, 2) + "\n");
