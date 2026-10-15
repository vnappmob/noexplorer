// Global app state shared across modules. Pane-specific state lives on each `Pane`.

import type { Config } from "./types";
import { store } from "./util";

export const app = {
  /** Saved connection, or null until the user sets one up. */
  cfg: null as Config | null,
  /** Bucket shown in the remote pane. */
  bucket: "",
  /** Show dotfiles in the local pane. */
  showHidden: store.get("hidden") === "1",
};
