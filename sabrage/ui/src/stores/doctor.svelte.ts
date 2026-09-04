// Doctor screen state: written by Doctor.svelte, read by the App shell for the
// sidebar failure badge. Module-scoped `$state`, no event bus, no persistence.

import { getAppState, runDoctor, type DoctorEvent, type DoctorSummary } from "../ipc";
import { errMsg } from "../lib/text";

/** One row as rendered — a `DoctorEvent` plus its streaming lifecycle. */
export interface DoctorRow extends DoctorEvent {
  /**
   * `"waiting"` — seen on a previous run but not yet reported by the current
   * one; the first waiting row stands in for "currently running" because the
   * backend streams resolved outcomes with no per-check start event.
   * `"done"` — the current run reported this slug.
   */
  phase: "waiting" | "done";
}

function createDoctorStore() {
  let rows = $state<DoctorRow[]>([]);
  let running = $state(false);
  let summary = $state<DoctorSummary | null>(null);
  let error = $state<string | null>(null);
  let hasRun = $state(false);
  /** Wall-clock timestamp of the last run's settlement (success or error) —
   * lets a screen re-mount skip re-firing a full pass when the last one is
   * still fresh. `null` until the first run settles. */
  let lastRunAtMs = $state<number | null>(null);
  let bottles = $state<string[]>([]);
  let bottlesLoaded = $state(false);
  /** `settings.json`'s `defaultBottle` as reported by `get_app_state` — the
   * Doctor screen's first choice before its hardcoded "Steam" fallback. */
  let defaultBottle = $state<string | null>(null);

  /** The one row (if any) standing in for "currently running" — `$derived` so
   * every row's read is an O(1) property access instead of each re-running
   * its own linear scan over `rows` per render. */
  const runningSlugValue = $derived.by((): string | null => {
    if (!running) return null;
    const next = rows.find((r) => r.phase === "waiting");
    return next?.slug ?? null;
  });

  async function loadBottles() {
    try {
      const state = await getAppState();
      bottles = state.bottles;
      defaultBottle = state.defaultBottle;
    } catch {
      bottles = [];
      defaultBottle = null;
    } finally {
      bottlesLoaded = true;
    }
  }

  async function run(args: { bottle?: string | null; bsDir?: string | null }) {
    running = true;
    error = null;
    summary = null;
    // Keep the previous run's rows as a skeleton (dimmed via `phase`) so a
    // rerun shows "waiting" placeholders instead of the list vanishing and
    // rebuilding from nothing; the very first run has no skeleton to reuse.
    rows = rows.map((r) => ({ ...r, phase: "waiting" }));

    try {
      const result = await runDoctor(args, (event) => {
        const idx = rows.findIndex((r) => r.slug === event.slug);
        const row: DoctorRow = { ...event, phase: "done" };
        if (idx >= 0) {
          rows[idx] = row;
        } else {
          rows.push(row);
        }
      });
      summary = result;
    } catch (e) {
      error = errMsg(e);
      // A rerun that rejects before reporting every slug must not leave the
      // previous run's rows dim forever with no explanation; rows this run did
      // report before rejecting stay.
      rows = rows.filter((r) => r.phase === "done");
    } finally {
      running = false;
      hasRun = true;
      lastRunAtMs = Date.now();
    }
  }

  return {
    get rows() {
      return rows;
    },
    get running() {
      return running;
    },
    get summary() {
      return summary;
    },
    get error() {
      return error;
    },
    get hasRun() {
      return hasRun;
    },
    get lastRunAtMs() {
      return lastRunAtMs;
    },
    get bottles() {
      return bottles;
    },
    get defaultBottle() {
      return defaultBottle;
    },
    get bottlesLoaded() {
      return bottlesLoaded;
    },
    /** FAIL count of the current summary — 0 whenever there is none (before the
     * first run, during a run, and after a rejected one). Drives the sidebar badge. */
    get failCount() {
      return summary?.failCount ?? 0;
    },
    get runningSlug() {
      return runningSlugValue;
    },
    loadBottles,
    run,
  };
}

/** Module-singleton store — one Doctor screen, one app window. */
export const doctorStore = createDoctorStore();
