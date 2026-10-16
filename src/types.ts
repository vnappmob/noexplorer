// Shapes shared with the Rust backend. Keep in sync with the `#[derive(Serialize)]` structs
// referenced in each comment.

/** `config::ProviderKind` — which provider an account uses. */
export type ProviderKind = "s3" | "nextcloud" | "webdav";

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
};

/** `config::General` — Settings → General. */
export type General = { run_in_background: boolean; check_updates: boolean; notify: boolean; auto_sizes: boolean };

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
  /** UI-only: `size` is a lower bound (some subfolders were skipped). */
  partial?: boolean;
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
  kind: "up" | "down";
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
};

export type SortBy = "name" | "size" | "modified";
