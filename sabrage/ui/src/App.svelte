<script lang="ts">
  /**
   * Application shell: owns the active `screen`, the Library entry EditGame is
   * open for, and the two menu-request counters. Owns no store — reads
   * `doctorStore` for the sidebar badge, calls `sessionStore.stop()` for Stop,
   * and passes `stageStore` to the stages panel and gate modal.
   *
   * Invariant: a menu-triggered Launch or Run Doctor only navigates and bumps
   * a counter here; the owning screen performs the action, giving one launch
   * path and one doctor path.
   */
  import { onMount } from "svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import GateModal from "./components/GateModal.svelte";
  import StagesPanel from "./components/StagesPanel.svelte";
  import QuitDialog from "./components/QuitDialog.svelte";
  import About from "./screens/About.svelte";
  import Library from "./screens/Library.svelte";
  import EditGame from "./screens/EditGame.svelte";
  import Session from "./screens/Session.svelte";
  import Doctor from "./screens/Doctor.svelte";
  import Logs from "./screens/Logs.svelte";
  import Settings from "./screens/Settings.svelte";
  import { onMenu } from "./ipc";
  import { doctorStore } from "./stores/doctor.svelte";
  import { sessionStore } from "./stores/session.svelte";
  import { stageStore } from "./stores/stage.svelte";
  import type { Screen } from "./types";

  let screen = $state<Screen>("about");
  /** The Library entry `"edit"` is open for; `null` means "Add game" (a new
   * entry). Only meaningful while `screen === "edit"`; stale otherwise. */
  let editGameId = $state<string | null>(null);

  /** Bumped every time the Pipeline ▸ Launch menu item (⌘R) fires; Session
   * watches the prop and calls its own `doLaunch(false)` once its bottle and
   * options have loaded, so the menu and the Launch button share one path. */
  let launchRequest = $state(0);

  /** Bumped every time the Pipeline ▸ Run Doctor menu item (⌘D) fires; Doctor
   * watches the prop and forces a fresh pass even when it is already the open
   * screen (plain navigation is then a no-op) or its cached result is fresh. */
  let doctorRequest = $state(0);

  function navigate(next: Screen) {
    screen = next;
  }

  /** Library's "Edit configuration…" — opens EditGame against a saved entry. */
  function editGame(id: string) {
    editGameId = id;
    screen = "edit";
  }

  /** Library's "Add game" — opens EditGame in new-entry mode. */
  function addGame() {
    editGameId = null;
    screen = "edit";
  }

  /** EditGame's Save/Cancel — both return to Library; on Save the entry has
   * already been persisted before this runs. */
  function doneEditing() {
    screen = "library";
  }

  // `onMenu` delivers exactly the three Pipeline menu ids built in
  // src-tauri's `build_menu` (Run Doctor, Launch, Stop).
  onMount(() => {
    let unlisten: (() => void) | undefined;
    void onMenu((id: string) => {
      if (id === "doctor") {
        navigate("doctor");
        doctorRequest++;
      } else if (id === "launch") {
        navigate("session");
        launchRequest++;
      } else if (id === "stop") void sessionStore.stop();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  });
</script>

<div class="app-shell">
  <Sidebar {screen} onNavigate={navigate} doctorBadge={doctorStore.failCount > 0} />

  <main class="main-area">
    {#if screen === "about"}
      <About onNavigate={navigate} />
    {:else if screen === "library"}
      <Library onNavigate={navigate} onEdit={editGame} onAdd={addGame} />
    {:else if screen === "edit"}
      <EditGame gameId={editGameId} onDone={doneEditing} onNavigate={navigate} />
    {:else if screen === "session"}
      <Session onNavigate={navigate} {launchRequest} />
    {:else if screen === "doctor"}
      <Doctor {doctorRequest} />
    {:else if screen === "logs"}
      <Logs />
    {:else if screen === "settings"}
      <Settings />
    {/if}
  </main>

  <StagesPanel open={stageStore.stagesPanelOpen} onClose={stageStore.closeStagesPanel} />
  <GateModal request={stageStore.gate} onClose={stageStore.closeGate} onNavigate={navigate} />
  <QuitDialog />
</div>

<style>
  .app-shell {
    display: flex;
    height: 100vh;
    min-height: 0;
    font-family: var(--font-body);
    color: var(--color-text);
    background: var(--color-bg);
    position: relative;
    font-size: 14px;
    line-height: 1.5;
  }
  .main-area {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
</style>
