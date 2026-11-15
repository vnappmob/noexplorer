// Global app state shared across modules. Pane-specific state lives on each `Pane`.

import type { Experimental, IPhoneDevice, Profile, Settings } from "./types";
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
  /** iPhones / iPads (experimental): connected, or with a backup here (devices.rs). */
  devices: [] as IPhoneDevice[],
  /** Show dotfiles in the local pane. */
  showHidden: store.get("hidden") === "1",
};

/** The default account (tray ✓, new views), or null before the first account is added. Each remote
 * view has its own account: use `pane.profile()` for anything about a view. */
export function activeProfile(): Profile | null {
  const s = app.settings;
  return s.profiles.find((p) => p.id === s.active) ?? s.profiles[0] ?? null;
}

/** Account ids of devices: `iphone:<UDID>` (devices.rs). */
export const DEVICE_PREFIX = "iphone:";
export const isDevice = (account: string | null | undefined) => !!account?.startsWith(DEVICE_PREFIX);

/** A saved account, or a device's (no saved profile: built from the Devices list). Null when it's
 * gone, or a device while the iPhone experiment is off. */
export function profileOf(id: string | null | undefined): Profile | null {
  if (!id) return null;
  if (!isDevice(id)) return app.settings.profiles.find((p) => p.id === id) ?? null;
  if (!exp("iphone")) return null;
  const udid = id.slice(DEVICE_PREFIX.length);
  const d = app.devices.find((x) => x.udid === udid);
  return {
    id,
    kind: "iphone",
    name: d?.name ?? deviceNames()[udid] ?? "iPhone",
    endpoint: udid,
    region: "",
    access_key: "",
    secret_key: "",
    bucket: "",
    path_style: true,
    cert_pin: "",
    project: "",
    key_file: "",
  };
}

/** Device names seen before, for a view on a device that isn't listed any more. */
export function deviceNames(): Record<string, string> {
  try {
    return JSON.parse(store.get("deviceNames") ?? "{}");
  } catch {
    return {};
  }
}

export function rememberDevices(list: IPhoneDevice[]) {
  const names = deviceNames();
  for (const d of list) names[d.udid] = d.name;
  store.set("deviceNames", JSON.stringify(names));
}

/** Is experimental feature `k` turned on (Settings → Experimental)? */
export const exp = (k: keyof Experimental) => !!app.settings.general.experimental?.[k];
