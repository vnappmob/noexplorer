// Global app state shared across modules. Pane-specific state lives on each `Pane`.

import type { Profile, Settings } from "./types";
import { store } from "./util";

export const app = {
  /** Saved accounts (from the backend's config.json). */
  settings: {
    profiles: [],
    active: "",
    download_dir: "",
    general: { run_in_background: true, check_updates: true, notify: true },
  } as Settings,
  /** Bucket shown in the remote pane. */
  bucket: "",
  /** Show dotfiles in the local pane. */
  showHidden: store.get("hidden") === "1",
};

/** The account shown in the remote pane, or null before the first account is added. */
export function activeProfile(): Profile | null {
  const s = app.settings;
  return s.profiles.find((p) => p.id === s.active) ?? s.profiles[0] ?? null;
}

/** localStorage key scoped to the active account (last bucket / prefix are per account). */
export const profileKey = (k: string) => `${k}@${activeProfile()?.id ?? ""}`;
