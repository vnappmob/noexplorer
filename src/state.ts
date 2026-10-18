// Global app state shared across modules. Pane-specific state lives on each `Pane`.

import type { Experimental, Profile, Settings } from "./types";
import { store } from "./util";

export const app = {
  /** Saved accounts (from the backend's config.json). */
  settings: {
    profiles: [],
    active: "",
    download_dir: "",
    general: {
      run_in_background: true,
      check_updates: true,
      auto_sizes: true,
      experimental: {} as Experimental,
      speed_limit_kbps: 0,
    },
  } as Settings,
  /** Show dotfiles in the local pane. */
  showHidden: store.get("hidden") === "1",
};

/** The default account (tray ✓, new views), or null before the first account is added. Each remote
 * view has its own account: use `pane.profile()` for anything about a view. */
export function activeProfile(): Profile | null {
  const s = app.settings;
  return s.profiles.find((p) => p.id === s.active) ?? s.profiles[0] ?? null;
}

/** Is experimental feature `k` turned on (Settings → Experimental)? */
export const exp = (k: keyof Experimental) => !!app.settings.general.experimental?.[k];
