// Cross-component state for the pipeline stage runner: the stage (if any) open
// in GateModal, and StagesPanel visibility. Module-scoped Svelte 5 rune store,
// same shape as doctor.svelte.ts.
//
// GateModal mounts once at the app root (App.svelte) and reads `gate` directly;
// every opener goes through `openGate` instead of mounting its own dialog.

import type { LaunchOpts, Stage } from "../ipc";

export interface GateRequest {
  stage: Stage;
  bottle?: string | null;
  bsDir?: string | null;
  dryRun?: boolean;
  /**
   * Present only when `stage === "run"`. The opener has already called
   * `sessionStore.launch(launch)`, so GateModal reads `sessionStore.launchRows` /
   * `sessionStore.launching` instead of calling `runStage`; `launch` is kept only
   * to re-issue the same launch from a Retry button after a Fatal row.
   */
  launch?: LaunchOpts;
  /**
   * Called once when the run settles: success, a Fatal row, or an `invoke()`
   * rejection. A Fix retry inside the same gate settles again and gets its own
   * call. Unused for `stage === "run"`, where the session store's own reactive
   * state is the settlement signal.
   */
  onFinished?: () => void;
}

function createStageStore() {
  let stagesPanelOpen = $state(false);
  let gate = $state<GateRequest | null>(null);
  /**
   * True while GateModal has a `runStage` call in flight for a non-run stage
   * (set via `setRunning`). Hide clears `gate` but not this, so openGate callers
   * must disable on it — see GateModal's `activeRequest` split, the other guard.
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
