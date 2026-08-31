// App-wide Settings state (`~/Library/Application Support/Sabrage/settings.json`),
// shared between the Settings screen (the writer) and every screen that reads
// a default (Sidebar's bottle prefill, Session's/Library's launch defaults).
// A plain Svelte 5 rune store — module-scoped `$state`, same shape as
// doctor.svelte.ts/session.svelte.ts.

import { errMsg } from "../lib/text";
import {
  getSettings as ipcGetSettings,
  saveSettings as ipcSaveSettings,
  type Settings,
} from "../ipc";

function createSettingsStore() {
  let settings = $state<Settings | null>(null);
  let loaded = $state(false);
  let loadOk = $state(false);
  let error = $state<string | null>(null);

  /** Serializes every write (`save`/`update`) onto one chain so overlapping
   * autosaves settle in call order — at most one `saveSettings` round-trip is
   * ever in flight, and each queued write merges/re-converges against the
   * result of the one before it rather than a snapshot captured before that
   * write even started. A queued step's own rejection does not break the
   * chain for callers still waiting behind it; it still rejects to its own
   * caller. */
  let chain: Promise<void> = Promise.resolve();
  function enqueue<T>(fn: () => Promise<T>): Promise<T> {
    const run = chain.then(fn, fn);
    chain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  /** Fetch `Settings` from disk. A missing file resolves to field defaults
   * (not an error — see `getSettings`'s doc comment). A corrupt file or IPC
   * failure quarantines the store instead of retaining a stale snapshot:
   * `settings` is cleared and `loadOk` drops to `false`, so every control
   * gated on `loadOk` (not `loaded` — see that getter's doc) disables rather
   * than autosaving over a value nothing here has verified is current, and a
   * later successful `load()`/`save()` is what re-arms them. */
  async function load(): Promise<void> {
    error = null;
    try {
      const next = await ipcGetSettings();
      settings = next;
      loadOk = true;
    } catch (e) {
      settings = null;
      loadOk = false;
      error = errMsg(e);
    } finally {
      loaded = true;
    }
  }

  /**
   * Persist `next` optimistically — `settings` updates immediately, before
   * the round-trip resolves, so a bound control never visibly snaps back
   * while the save is in flight. On rejection, `error` is set and the store
   * re-`load()`s from disk rather than rolling back to a `previous` snapshot
   * captured before this call: under `enqueue`'s serialization a rollback is
   * still safe against *this* store's own writes, but not against a write
   * from another process (setup, the CLI) landing in between — re-reading is
   * the only way `settings` converges on what is actually on disk. Queued
   * through `enqueue`, so a direct call still serializes against `update`.
   */
  async function save(next: Settings): Promise<void> {
    return enqueue(() => performSave(next));
  }

  async function performSave(next: Settings): Promise<void> {
    settings = next;
    error = null;
    try {
      settings = await ipcSaveSettings(next);
      loadOk = true;
    } catch (e) {
      error = errMsg(e);
      await load();
      throw e;
    }
  }

  /**
   * Shallow-merge `patch` onto the current `settings` and save the result,
   * serialized against every other in-flight `update`/`save` (see
   * `enqueue`). Rejects — rather than silently no-oping — when nothing has
   * successfully loaded yet: the old no-op let a caller flash "Saved" for a
   * write that never happened (every autosave handler in Settings.svelte
   * awaits this and only flashes on success, so the rejection now suppresses
   * that). `patch.launch`, if present, replaces the whole `LaunchDefaults`
   * object (this merge is shallow) — pass a full `{ ...settings.launch, … }`
   * object to change one flag.
   */
  async function update(patch: Partial<Settings>): Promise<void> {
    return enqueue(() => {
      if (!settings) {
        return Promise.reject(new Error("settings not loaded — reload Settings and try again"));
      }
      return performSave({ ...settings, ...patch });
    });
  }

  return {
    get settings() {
      return settings;
    },
    get loaded() {
      return loaded;
    },
    /** `true` only after a load that actually succeeded. `loaded` alone
     * (kept for "a load has been attempted") is not safe to gate controls
     * on: a corrupt/unreadable settings.json also sets it, and gating there
     * used to leave every control enabled over a `null` `settings` — see
     * `load`'s doc comment. */
    get loadOk() {
      return loadOk;
    },
    get error() {
      return error;
    },
    load,
    save,
    update,
  };
}

/** Module-singleton store — one app window, one settings file. */
export const settingsStore = createSettingsStore();
