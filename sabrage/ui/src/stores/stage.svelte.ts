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

import type { Stage } from "../ipc";

export interface GateRequest {
  stage: Stage;
  bottle?: string | null;
  bsDir?: string | null;
  dryRun?: boolean;
  /**
   * Called once the run settles — successfully, with a Fatal, or via an
   * invoke() rejection (a cancelled run still settles one of these ways).
   * Not called again on a later Fix retry from within the same gate; that
   * retry is its own settlement and gets its own call. Doctor.svelte uses
   * this to re-run its checks after a whole-stage fix completes.
   */
  onFinished?: () => void;
}

function createStageStore() {
  let stagesPanelOpen = $state(false);
  let gate = $state<GateRequest | null>(null);

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

  return {
    get stagesPanelOpen() {
      return stagesPanelOpen;
    },
    get gate() {
      return gate;
    },
    openStagesPanel,
    closeStagesPanel,
    openGate,
    closeGate,
  };
}

/** Module-singleton store — one app window, one pipeline runner. */
export const stageStore = createStageStore();
