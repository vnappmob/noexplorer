// File-type detection used for icons and the quick-look preview.

/** URL of a local file for <img>/<video> (proxy.rs, the same nxr:// scheme remote previews use). */
export const localSrc = (path: string) =>
  `${navigator.userAgent.includes("Windows") ? "http://nxr.localhost" : "nxr://localhost"}/~local/f/${encodeURIComponent(path)}`;

export const ext = (name: string) => (name.includes(".") ? name.split(".").pop()!.toLowerCase() : "");

/** Extension lists per preview kind. `text` must match `util::is_text_like` in Rust. */
export const KIND: Record<string, string[]> = {
  image: ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico", "avif", "heic", "heif", "tif", "tiff"],
  video: ["mp4", "m4v", "mov", "webm", "ogv"],
  audio: ["mp3", "wav", "m4a", "aac", "ogg", "oga", "flac", "opus"],
  pdf: ["pdf"],
  text: [
    "txt",
    "md",
    "log",
    "json",
    "yaml",
    "yml",
    "toml",
    "ini",
    "conf",
    "cfg",
    "env",
    "csv",
    "tsv",
    "xml",
    "sql",
    "sh",
    "bash",
    "zsh",
    "py",
    "js",
    "ts",
    "tsx",
    "jsx",
    "rs",
    "go",
    "java",
    "kt",
    "swift",
    "c",
    "h",
    "cpp",
    "hpp",
    "cs",
    "php",
    "rb",
    "html",
    "css",
    "scss",
    "vue",
    "dart",
    "gradle",
    "properties",
    "dockerfile",
    "gitignore",
  ],
  archive: ["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "dmg", "iso"],
};

export function kindOf(name: string) {
  const e = ext(name);
  for (const [k, list] of Object.entries(KIND)) if (list.includes(e)) return k;
  return "other";
}

export const ICON: Record<string, string> = {
  image: "🖼",
  video: "🎬",
  audio: "🎵",
  pdf: "📕",
  text: "📄",
  archive: "🗜",
  other: "📄",
};
