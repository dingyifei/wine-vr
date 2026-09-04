// `AppState` from `get_app_state` (one Tauri round trip), shared by Session,
// Settings, EditGame, StagesPanel, and Sidebar (for `alvrVersion`). Doctor is
// not a consumer — doctorStore fetches its own copy. Module-scoped Svelte 5
// rune store, same shape as doctor.svelte.ts and session.svelte.ts.
//
// `load()` re-fetches every call (not a cross-screen cache); concurrent callers
// within the same tick share one in-flight request.

import { getAppState, type AppState } from "../ipc";

function createBottlesStore() {
  let state = $state<AppState | null>(null);
  let bottlesLoaded = $state(false);
  let inFlight: Promise<AppState | null> | null = null;

  async function load(): Promise<AppState | null> {
    if (inFlight) return inFlight;
    inFlight = (async () => {
      try {
        const next = await getAppState();
        state = next;
        return next;
      } catch {
        state = null;
        return null;
      } finally {
        bottlesLoaded = true;
        inFlight = null;
      }
    })();
    return inFlight;
  }

  return {
    get state() {
      return state;
    },
    get bottles() {
      return state?.bottles ?? [];
    },
    get bottlesLoaded() {
      return bottlesLoaded;
    },
    /** `settings.json`'s `defaultBottle` as reported by `get_app_state`. */
    get defaultBottle() {
      return state?.defaultBottle ?? null;
    },
    get defaultBsDir() {
      return state?.defaultBsDir ?? null;
    },
    get alvrVersion() {
      return state?.alvrVersion ?? null;
    },
    load,
  };
}

/** Module-singleton store — one app window, one bottle list. */
export const bottlesStore = createBottlesStore();
