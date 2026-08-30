// App-wide Settings state (`~/Library/Application Support/Sabrage/settings.json`),
// shared between the Settings screen (the writer) and every screen that reads
// a default (Sidebar's bottle prefill, Session's/Library's launch defaults).
// A plain Svelte 5 rune store — module-scoped `$state`, same shape as
// doctor.svelte.ts/session.svelte.ts.

import {
  getSettings as ipcGetSettings,
  saveSettings as ipcSaveSettings,
  type Settings,
} from "../ipc";

function createSettingsStore() {
  let settings = $state<Settings | null>(null);
  let loaded = $state(false);
  let error = $state<string | null>(null);

  /** Fetch `Settings` from disk. A missing file resolves to field defaults
   * (not an error — see `getSettings`'s doc comment); only a corrupt file or
   * IPC failure sets `error`, leaving `settings` at whatever it held before
   * (`null` on the very first load). */
  async function load(): Promise<void> {
    error = null;
    try {
      settings = await ipcGetSettings();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loaded = true;
    }
  }

  /**
   * Persist `next` optimistically — `settings` updates immediately, before
   * the round-trip resolves, so a bound control never visibly snaps back
   * while the save is in flight. On rejection, `settings` rolls back to
   * whatever it held before this call and `error` is set; the caller (an
   * autosave control, typically) still sees the rejection via the rethrow
   * and may show its own transient failure indicator on top of the rollback.
   */
  async function save(next: Settings): Promise<void> {
    const previous = settings;
    settings = next;
    error = null;
    try {
      settings = await ipcSaveSettings(next);
    } catch (e) {
      settings = previous;
      error = e instanceof Error ? e.message : String(e);
      throw e;
    }
  }

  /**
   * Shallow-merge `patch` onto the current `settings` and save the result.
   * A no-op (never throws, never touches `error`) when nothing has loaded
   * yet — callers only enable update-triggering controls once `loaded` is
   * true, so this guards a startup race rather than a real user action.
   * `patch.launch`, if present, replaces the whole `LaunchDefaults` object
   * (this merge is shallow) — pass a full `{ ...settings.launch, … }` object
   * to change one flag.
   */
  async function update(patch: Partial<Settings>): Promise<void> {
    if (!settings) return;
    await save({ ...settings, ...patch });
  }

  return {
    get settings() {
      return settings;
    },
    get loaded() {
      return loaded;
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
