// The saved-games library (`~/Library/Application Support/Sabrage/library.json`),
// shared between the Library screen (the list) and EditGame (one entry at a
// time). A plain Svelte 5 rune store — module-scoped `$state`, same shape as
// doctor.svelte.ts/session.svelte.ts. `GameRow.validity` is recomputed by the
// backend on every fetch (never persisted), so `rows` is only ever as fresh
// as the last `refresh()`/`save()`/`remove()` — there is no standing
// subscription here, unlike session.svelte.ts's status broadcast.

import { errMsg } from "../lib/text";
import {
  getLibrary as ipcGetLibrary,
  removeGame as ipcRemoveGame,
  saveGame as ipcSaveGame,
  type GameEntry,
  type GameRow,
} from "../ipc";

function createLibraryStore() {
  let rows = $state<GameRow[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);

  /** Re-fetch every row (entry + freshly computed validity) from disk. */
  async function refresh(): Promise<void> {
    loading = true;
    error = null;
    try {
      rows = await ipcGetLibrary();
    } catch (e) {
      error = errMsg(e);
    } finally {
      loading = false;
    }
  }

  /**
   * Upsert `entry` by `entry.id` and return the stored row; an existing row
   * stays in place, a new one is appended. Leaves `loading`/`error`
   * untouched — EditGame renders its own inline failure.
   */
  async function save(entry: GameEntry): Promise<GameRow> {
    const row = await ipcSaveGame(entry);
    const idx = rows.findIndex((r) => r.entry.id === row.entry.id);
    if (idx >= 0) {
      rows[idx] = row;
    } else {
      rows.push(row);
    }
    return row;
  }

  /** Remove the entry named `id` and drop it from `rows` when the backend
   * confirms removal (a `false` return — already gone — leaves `rows`
   * untouched rather than silently vanishing a row that was never there). */
  async function remove(id: string): Promise<boolean> {
    const removed = await ipcRemoveGame(id);
    if (removed) {
      rows = rows.filter((r) => r.entry.id !== id);
    }
    return removed;
  }

  /** The current row for `id`, from whatever `rows` last held — `undefined`
   * before the first `refresh()` or if `id` names no saved entry. */
  function byId(id: string): GameRow | undefined {
    return rows.find((r) => r.entry.id === id);
  }

  return {
    get rows() {
      return rows;
    },
    get loading() {
      return loading;
    },
    get error() {
      return error;
    },
    refresh,
    save,
    remove,
    byId,
  };
}

/** Module-singleton store — one app window, one library file. */
export const libraryStore = createLibraryStore();
