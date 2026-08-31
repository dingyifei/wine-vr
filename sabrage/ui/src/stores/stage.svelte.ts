// Shared cross-component state for the pipeline stage runner: which stage (if
// any) is open in the GateModal, and the StagesPanel's visibility. A plain
// Svelte 5 rune store, module-scoped — the same shape as doctor.svelte.ts.
//
// GateModal is mounted once at the app root (App.svelte) and reads `gate`
// directly; any component may open one by calling `openGate` — the Doctor
// screen's whole-stage Fix buttons, StagesPanel's Run/Dry-run buttons, and
// GateModal itself (a Fatal row's Fix button, when the fix maps to a whole
// other stage) all go through this one entry point instead of each owning a
// separate dialog instance.

import type { LaunchOpts, Stage } from "../ipc";

export interface GateRequest {
  stage: Stage;
  bottle?: string | null;
  bsDir?: string | null;
  dryRun?: boolean;
  /**
   * Present only when `stage === "run"`. `sessionStore.launch(launch)` has
   * already been called (by whoever opened this gate — the Session screen's
   * Launch/Dry-run buttons) by the time the modal mounts; GateModal does NOT
   * call `runStage` for this stage — it reads `sessionStore.launchRows` /
   * `sessionStore.launching` instead, and uses `launch` only to re-issue the
   * same launch from a Retry button after a Fatal row.
   */
  launch?: LaunchOpts;
  /**
   * Called once the run settles — successfully, with a Fatal, or via an
   * invoke() rejection (a cancelled run still settles one of these ways).
   * Not called again on a later Fix retry from within the same gate; that
   * retry is its own settlement and gets its own call. Doctor.svelte uses
   * this to re-run its checks after a whole-stage fix completes. Unused for
   * `stage === "run"` — the session store's own reactive state is the
   * settlement signal there.
   */
  onFinished?: () => void;
}

function createStageStore() {
  let stagesPanelOpen = $state(false);
  let gate = $state<GateRequest | null>(null);
  /**
   * True while GateModal has an actual `runStage` invocation in flight for a
   * non-run stage (setup/build/install/stop) — mirrors that component's own
   * `running` local, set/cleared from there via `setRunning`. Distinct from
   * `gate !== null`: Hide clears `gate` (and hides the dialog) without
   * touching this, so callers that would otherwise silently queue a second
   * `openGate` on top of a still-running, merely-hidden stage (StagesPanel's
   * Run/Dry-run, Doctor's whole-stage Fix) can disable themselves on it
   * instead. See GateModal's `activeRequest`/`displayRequest` split, which is
   * the other half of this fix — this flag is defence in depth, not the only
   * guard.
   */
  let running = $state(false);

  function openStagesPanel() {
    stagesPanelOpen = true;
  }
  function closeStagesPanel() {
    stagesPanelOpen = false;
  }

  function openGate(req: GateRequest) {
    gate = req;
  }
  function closeGate() {
    gate = null;
  }
  function setRunning(v: boolean) {
    running = v;
  }

  return {
    get stagesPanelOpen() {
      return stagesPanelOpen;
    },
    get gate() {
      return gate;
    },
    get running() {
      return running;
    },
    openStagesPanel,
    closeStagesPanel,
    openGate,
    closeGate,
    setRunning,
  };
}

/** Module-singleton store — one app window, one pipeline runner. */
export const stageStore = createStageStore();
