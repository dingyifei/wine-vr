// Doctor screen state, shared between Doctor.svelte (the writer) and the App
// shell (the reader, for the sidebar failure badge). A plain Svelte 5 rune
// store — module-scoped `$state`, no event bus, no persistence. Phase 1 keeps
// this to exactly what the Doctor screen and the sidebar badge need; a
// broader cross-screen event bus is explicitly future work (see the App
// agent's task brief).

import { getAppState, runDoctor, type DoctorEvent, type DoctorSummary } from "../ipc";
import { errMsg } from "../lib/text";

/** One row as rendered — a `DoctorEvent` plus its streaming lifecycle. */
export interface DoctorRow extends DoctorEvent {
  /**
   * `"waiting"` — this slug was seen on a previous run but hasn't reported in
   * for the current one yet (rendered dim, empty-square icon; the *first*
   * waiting row also gets the spinner, standing in for "currently running"
   * since the backend streams already-resolved outcomes with no separate
   * start event per check).
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
  /** `settings.json`'s `defaultBottle` as reported by `get_app_state` (Phase 4)
   * — the Doctor screen's first choice before its hardcoded "Steam" fallback. */
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
      // PREVIOUS run's rows sitting dim forever with no explanation — drop
      // every placeholder this run never got to reporting on. Rows this run
      // DID report before rejecting (`phase === "done"`, set by the callback
      // above) stay.
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
    /** Sidebar badge: a completed run found at least one FAIL. */
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
