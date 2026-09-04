// The runtime config editor's state (`oxrsys-runtime.toml`, via
// `read_runtime_config`/`write_runtime_config`), owned by the Settings
// screen's Streaming card. A plain Svelte 5 rune store — module-scoped
// `$state`, same shape as doctor.svelte.ts/session.svelte.ts.

import { errMsg } from "../lib/text";
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

  /** Fetch the current `RuntimeConfigView` into `view`; never rejects.
   * A missing `oxrsys-runtime.toml` yields `view.exists` `false` with every value `null`.
   * An IPC-layer failure leaves `view` untouched and sets `error`. */
  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      view = await ipcReadRuntimeConfig();
    } catch (e) {
      error = errMsg(e);
    } finally {
      loading = false;
    }
  }

  /**
   * Apply `patch`, re-load `view`, and return the backend's `WriteReport`.
   * Re-loading keeps the backend the source of truth: a write can create the
   * file from template, resolve `shadowed` occurrences, or change `modifiedUnixMs`.
   * Rejects and sets `error` on a `parseError`, failed validation, or a live
   * session; the Settings screen validates client-side first, so this is the backstop.
   */
  async function write(patch: RuntimeConfigPatch): Promise<WriteReport> {
    error = null;
    try {
      const report = await ipcWriteRuntimeConfig(patch);
      await load();
      return report;
    } catch (e) {
      error = errMsg(e);
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
