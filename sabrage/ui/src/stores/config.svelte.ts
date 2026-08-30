// The runtime config editor's state (`oxrsys-runtime.toml`, via
// `read_runtime_config`/`write_runtime_config`), owned by the Settings
// screen's Streaming card. A plain Svelte 5 rune store — module-scoped
// `$state`, same shape as doctor.svelte.ts/session.svelte.ts.

import {
  readRuntimeConfig as ipcReadRuntimeConfig,
  writeRuntimeConfig as ipcWriteRuntimeConfig,
  type RuntimeConfigPatch,
  type RuntimeConfigView,
  type WriteReport,
} from "../ipc";

function createConfigStore() {
  let view = $state<RuntimeConfigView | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  /** Fetch the current `RuntimeConfigView`. Never rejects for a missing
   * `oxrsys-runtime.toml` (`view.exists` is `false`, every value `null`) —
   * only a genuine IPC-layer failure sets `error`. */
  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      view = await ipcReadRuntimeConfig();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  /**
   * Apply `patch` and re-fetch `view` from the result — a write can create
   * the file from template, resolve `shadowed` occurrences, or change
   * `modifiedUnixMs`, so the backend's own reader stays the one source of
   * truth for the resulting shape rather than patching `view` locally.
   * Rejects (and sets `error`) when the file has a `parseError` or the patch
   * fails validation — the Settings screen should already have caught an
   * invalid value client-side before calling this (`validate`-shaped UI, not
   * a surprise here), but this is the backstop.
   */
  async function write(patch: RuntimeConfigPatch): Promise<WriteReport> {
    error = null;
    try {
      const report = await ipcWriteRuntimeConfig(patch);
      await load();
      return report;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      throw e;
    }
  }

  return {
    get view() {
      return view;
    },
    get loading() {
      return loading;
    },
    get error() {
      return error;
    },
    load,
    write,
  };
}

/** Module-singleton store — one app window, one runtime config file. */
export const configStore = createConfigStore();
