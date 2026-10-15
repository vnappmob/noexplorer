// Shapes shared with the Rust backend. Keep in sync with the `#[derive(Serialize)]` structs
// referenced in each comment.

/** `config::Profile` — one S3 account. */
export type Profile = {
  id: string;
  name: string;
  endpoint: string;
  region: string;
  access_key: string;
  secret_key: string;
  bucket: string;
  path_style: boolean;
};

/** `config::General` — Settings → General. */
export type General = { run_in_background: boolean; check_updates: boolean; notify: boolean };

/** `config::Settings` — everything in config.json. */
export type Settings = { profiles: Profile[]; active: string; download_dir: string; general: General };

/** `config::BucketInfo` — an entry of the bucket picker. */
export type BucketInfo = { name: string; region: string | null };

/** `remote::SizeInfo` — result of `remote_size` / `local_size`. */
export type SizeInfo = { bytes: number; files: number };

/** `remote::Entry` — a row in either pane. `key` is the S3 key (remote) or absolute path (local). */
export type Item = {
  key: string;
  name: string;
  size: number;
  modified: number | null;
  dir: boolean;
  /** UI-only: a folder whose `size` was computed with "Σ Size" (folders are listed as 0). */
  calc?: boolean;
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
