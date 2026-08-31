// The `AppState` blob (`get_app_state` — one Tauri round trip) shared by
// every screen that needs the bottle list: Session, Settings, EditGame,
// StagesPanel, Doctor (via doctorStore, which composes this), and the
// Sidebar (for `alvrVersion`). Previously each of those screens ran its own
// `getAppState()` call and re-derived `bottles`/`bottlesLoaded` locally —
// same shape, five separate copies. A plain Svelte 5 rune store —
// module-scoped `$state`, same shape as doctor.svelte.ts/session.svelte.ts.
//
// `load()` always performs a fresh round trip (matching every call site's
// previous per-mount behavior — this does not cache across screens, it only
// removes the duplicated fetch/`bottlesLoaded` bookkeeping). Concurrent
// callers within the same tick collapse onto one in-flight request.

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
