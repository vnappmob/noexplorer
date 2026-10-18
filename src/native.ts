// Native window look (experimental "native_look"): macOS translucency or Windows 11 Mica behind the
// panes. The window is transparent on macOS and Windows (tauri.<os>.conf.json), but the page stays
// opaque unless the effect really applied (`html.native`), so nothing changes when it's off or
// unsupported (Windows 10, Linux).

import { Effect, EffectState, getCurrentWindow } from "@tauri-apps/api/window";
import { exp } from "./state";

const isMac = navigator.userAgent.includes("Mac");
const isWin = navigator.userAgent.includes("Windows");

export async function applyNativeLook() {
  const root = document.documentElement;
  const w = getCurrentWindow();
  if (!exp("native_look") || !(isMac || isWin)) {
    root.classList.remove("native");
    await w.clearEffects().catch(() => {});
    return;
  }
  try {
    await w.setEffects({
      effects: isMac ? [Effect.UnderWindowBackground] : [Effect.Mica],
      state: EffectState.FollowsWindowActiveState,
    });
    root.classList.add("native");
  } catch {
    root.classList.remove("native");
  }
}

export function setupNativeLook() {
  window.addEventListener("experimental", applyNativeLook);
  window.addEventListener("settings-loaded", applyNativeLook);
}
