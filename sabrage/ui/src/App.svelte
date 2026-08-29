<script lang="ts">
  import { onMount } from "svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import GateModal from "./components/GateModal.svelte";
  import StagesPanel from "./components/StagesPanel.svelte";
  import QuitDialog from "./components/QuitDialog.svelte";
  import About from "./screens/About.svelte";
  import Library from "./screens/Library.svelte";
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

  function navigate(next: Screen) {
    screen = next;
  }

  // Pipeline menu items that map onto navigation/actions this shell already
  // owns (Run Doctor, Launch, Stop); anything else (Setup…/Build/Install…,
  // Open Logs/Config Folder) is another screen's or the opener plugin's job
  // and is deliberately a no-op here rather than guessed at.
  onMount(() => {
    let unlisten: (() => void) | undefined;
    void onMenu((id: string) => {
      if (id === "doctor") navigate("doctor");
      else if (id === "launch") navigate("session");
      else if (id === "stop") void sessionStore.stop();
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
      <Library />
    {:else if screen === "session"}
      <Session onNavigate={navigate} />
    {:else if screen === "doctor"}
      <Doctor />
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
