// Shapes shared with the Rust backend. Keep in sync with the `#[derive(Serialize)]` structs
// referenced in each comment.

/** `config::Config` */
export type Config = {
  endpoint: string;
  region: string;
  access_key: string;
  secret_key: string;
  bucket: string;
  path_style: boolean;
  download_dir: string;
};

/** `remote::Entry` — a row in either pane. `key` is the S3 key (remote) or absolute path (local). */
export type Item = { key: string; name: string; size: number; modified: number | null; dir: boolean };

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
