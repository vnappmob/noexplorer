// Shapes shared with the Rust backend. Keep in sync with the `#[derive(Serialize)]` structs
// referenced in each comment.

/** `config::ProviderKind` — which provider an account uses. */
export type ProviderKind = "s3" | "nextcloud" | "webdav" | "box" | "azure" | "sftp" | "onedrive";

/** `config::Profile` — one remote account. */
export type Profile = {
  id: string;
  /** Missing in older settings = "s3". */
  kind?: ProviderKind;
  name: string;
  endpoint: string;
  region: string;
  access_key: string;
  /** Always "" from the backend (secrets stay there); "" when saving = keep the saved one. */
  secret_key: string;
  bucket: string;
  path_style: boolean;
  /** The secret is in the OS credential store (Keychain…) rather than config.json. */
  keychain?: boolean;
  /** SHA-256 fingerprint of a trusted self-signed certificate ("" = normal checks). tls.rs. */
  cert_pin?: string;
  /** Google Cloud Storage: the project whose buckets are listed. */
  project?: string;
  /** SFTP: a private key file for `ssh -i` (optional). */
  key_file?: string;
};

/** `config::General` — Settings → General. */
export type General = {
  run_in_background: boolean;
  check_updates: boolean;
  auto_sizes: boolean;
  experimental: Experimental;
  /** Transfer speed limit in KB/s, 0 = no limit. */
  speed_limit_kbps: number;
};
/** Settings → Experimental (all off by default). Mirrors `Experimental` in config.rs. */
export type Experimental = {
  self_signed: boolean;
  backup_pairs: boolean;
  box_beta: boolean;
  bucket_policy: boolean;
};

/** `config::Settings` — everything in config.json. */
export type Settings = { profiles: Profile[]; active: string; download_dir: string; general: General };

/** `provider::BucketInfo` — an entry of the bucket picker. */
export type BucketInfo = { name: string; region: string | null };

/** `remote::SizeInfo` — result of `remote_size` / `local_size`. */
export type SizeInfo = { bytes: number; files: number };

/** `provider::Entry` — a row in either pane. `key` is the remote key/path (remote) or absolute path (local). */
export type Item = {
  key: string;
  name: string;
  size: number;
  modified: number | null;
  dir: boolean;
  /** UI-only: a folder whose `size` was computed (Σ, or automatic sizes; folders are listed as 0). */
  calc?: boolean;
  /** UI-only, search results: the folder it's in, relative to the folder searched (search.ts). */
  where?: string;
  /** UI-only: `size` is a lower bound (some subfolders were skipped). */
  partial?: boolean;
};

/** `compare::Diff` — result of `compare` (Compare dialog). */
export type Diff = {
  local_files: number;
  remote_files: number;
  only_local: { rel: string; size: number }[];
  only_remote: { rel: string; size: number }[];
  changed: { rel: string; local_size: number; remote_size: number; reason: "size" | "content" }[];
  junk: { rel: string; size: number }[];
  symlinks: { rel: string; size: number }[];
  same_verified: number;
  same_size: number;
  errors: string[];
};

/** `local::VolumeInfo` — the disk under the local pane's folder (storage bar). */
export type VolumeInfo = { name: string; total: number; available: number };

/** `scan::Totals` — a folder and everything below it. `skipped` > 0 = a lower bound. */
export type Totals = { bytes: number; files: number; dirs: number; skipped: number };

/** `sizes::DirSizes` — `folder_sizes` result and the `"dir-sizes"` event. */
export type DirSizes = {
  folder: string;
  sizes: (Totals & { path: string })[];
  total: Totals | null;
  scanning: boolean;
  /** The folder's own entries changed on disk: list it again. */
  relist: boolean;
};

/** `local::Crumb` */
export type Crumb = { name: string; path: string };

/** `local::LocalListing` */
export type LocalListing = { path: string; parent: string | null; crumbs: Crumb[]; roots: Crumb[]; entries: Item[] };

/** `jobs::JobEvent` — payload of the `"job"` event. */
export type JobEvent = {
  id: string;
  kind: "up" | "down" | "copy" | "delete" | "move" | "dup";
  label: string;
  /** Profile id of the remote account. */
  account: string;
  bucket: string;
  prefix: string;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
  status: "running" | "done" | "error" | "cancelled";
  error: string | null;
  dest: string | null;
  local_dir: string | null;
  /** Copy jobs: where the files come from. */
  src_account: string | null;
  src_bucket: string | null;
  /** Uploads and copies: the folder the files come from (local path or source prefix). */
  src_dir: string | null;
  /** Paused from the UI (experimental resume). */
  paused: boolean;
};

export type SortBy = "name" | "size" | "modified";

/** A transfer that didn't finish last time (resume.rs `PendingJob`). */
export type PendingJob = { id: string; kind: "up" | "down"; label: string; bucket: string; prefix: string };
