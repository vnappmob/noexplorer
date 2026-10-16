// Picks the right installer for the visitor's OS from the latest *published* GitHub release.
// Drafts are invisible to this API, so a broken release never shows up here (ADR-012).
// Also: the version in the hero badge, and the Setup tabs.
const REPO = "vnappmob/noexplorer";
const RELEASES = `https://github.com/${REPO}/releases/latest`;

const OSES = {
  mac: { name: "macOS", patterns: [/universal\.dmg$/i, /\.dmg$/i] },
  windows: { name: "Windows", patterns: [/-setup\.exe$/i, /\.msi$/i] },
  linux: { name: "Linux", patterns: [/\.AppImage$/i, /\.deb$/i, /\.rpm$/i] },
};

function detectOS() {
  const p = (navigator.userAgentData?.platform || navigator.platform || navigator.userAgent).toLowerCase();
  if (p.includes("mac")) return "mac";
  if (p.includes("win")) return "windows";
  if (p.includes("linux") || p.includes("x11")) return "linux";
  return null;
}

const pick = (assets, os) => {
  for (const re of OSES[os].patterns) {
    const a = assets.find((x) => re.test(x.name));
    if (a) return a;
  }
  return null;
};
const mb = (n) => `${(n / 1048576).toFixed(0)} MB`;

async function main() {
  const os = detectOS();
  const main = document.getElementById("dl-main");
  const label = document.getElementById("dl-label");
  const meta = document.getElementById("dl-meta");
  const other = document.getElementById("dl-other");
  if (os) label.textContent = `Download for ${OSES[os].name}`;

  let release;
  try {
    const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!r.ok) throw new Error(String(r.status));
    release = await r.json();
  } catch {
    // No published release yet, or rate-limited: keep the button pointing at the Releases page.
    meta.textContent = "See all releases on GitHub";
    return;
  }

  const badge = document.getElementById("ver-badge");
  if (badge && release.tag_name)
    badge.insertAdjacentHTML("beforeend", ` <span class="ver">· ${release.tag_name}</span>`);

  const assets = release.assets || [];
  const mine = os && pick(assets, os);
  if (mine) {
    main.href = mine.browser_download_url;
    meta.textContent = `${release.tag_name} · ${mine.name.split(".").pop().toUpperCase()} · ${mb(mine.size)}`;
  } else {
    main.href = RELEASES;
    meta.textContent = `${release.tag_name} · all platforms`;
  }

  const links = Object.entries(OSES)
    .filter(([k]) => k !== os)
    .map(([k, v]) => {
      const a = pick(assets, k);
      return a ? `<a href="${a.browser_download_url}">${v.name}</a>` : "";
    })
    .filter(Boolean);
  other.innerHTML = `Also for ${links.join(" ")} · <a href="${RELEASES}">All downloads</a>`;
}

main();

// Setup tabs (S3 / Nextcloud / WebDAV / Box).
document.querySelectorAll(".tabs [data-tab]").forEach((tab) =>
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tabs [data-tab]").forEach((t) => t.setAttribute("aria-selected", String(t === tab)));
    document.querySelectorAll("[data-panel]").forEach((p) => (p.hidden = p.dataset.panel !== tab.dataset.tab));
  }),
);
