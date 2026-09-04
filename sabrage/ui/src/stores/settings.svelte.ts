// App-wide Settings state (`~/Library/Application Support/Sabrage/settings.json`),
// shared between the Settings screen (the writer) and every screen that reads
// a default (Sidebar's bottle prefill, Session's/Library's launch defaults).

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

  /** Write counter, bumped synchronously when `save`/`update` queues a write,
   * so it already reflects the caller's own write on return. If `writeSeq`
   * exceeds `capturedBefore + 1` once a write settles, a later write
   * superseded it and the result is stale. */
  let writeSeq = $state(0);

  /** Serializes every write (`save`/`update`) onto one chain, so overlapping
   * autosaves settle in call order: at most one `saveSettings` round-trip is
   * ever in flight, and each queued write merges against the result of the one
   * before it. A queued step's rejection reaches its own caller without
   * breaking the chain for writes behind it. */
  let chain: Promise<void> = Promise.resolve();
  function enqueue<T>(fn: () => Promise<T>): Promise<T> {
    writeSeq++;
    const run = chain.then(fn, fn);
    chain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  /** Fetch `Settings` from disk. A missing file resolves to field defaults,
   * not an error. A corrupt file or IPC failure quarantines the store —
   * `settings` cleared, `loadOk` false — so controls gated on `loadOk` disable
   * rather than autosave over a value nothing has verified; a later successful
   * `load()`/`save()` re-arms them. */
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
   * Persist `next` optimistically: `settings` updates before the round-trip
   * resolves, so bound controls never snap back. On rejection `error` is set
   * and the store re-`load()`s from disk; it does not roll back, because that
   * would discard a write another process (setup, the CLI) landed in between,
   * and the UI has no test harness to catch such interleavings. Queued through
   * `enqueue`, so a direct call still serializes against `update`.
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
   * Shallow-merge `patch` onto `settings` and save, serialized through
   * `enqueue`. Rejects when nothing has loaded yet, so callers cannot report
   * success for a write that never happened. `patch.launch` replaces the whole
   * `LaunchDefaults` object — pass `{ ...settings.launch, … }` to change one
   * flag.
   *
   * `patch` may be a function of the current `settings`, resolved inside the
   * queued step against whatever `settings` is when this write runs. Prefer
   * this form when building a patch from a nested sub-object (e.g. `launch`):
   * otherwise a field captured before an earlier write's failure-triggered
   * `load()` rides along into this write.
   */
  async function update(patch: Partial<Settings> | ((current: Settings) => Partial<Settings>)): Promise<void> {
    return enqueue(() => {
      if (!settings) {
        return Promise.reject(new Error("settings not loaded — reload Settings and try again"));
      }
      const resolved = typeof patch === "function" ? patch(settings) : patch;
      return performSave({ ...settings, ...resolved });
    });
  }

  return {
    get settings() {
      return settings;
    },
    get loaded() {
      return loaded;
    },
    /** `true` only after a load that actually succeeded. Gate controls on this,
     * not on `loaded` ("a load was attempted"), which a corrupt or unreadable
     * settings.json also sets while `settings` stays `null`. */
    get loadOk() {
      return loadOk;
    },
    get error() {
      return error;
    },
    get writeSeq() {
      return writeSeq;
    },
    load,
    save,
    update,
  };
}

/** Module-singleton store — one app window, one settings file. */
export const settingsStore = createSettingsStore();
