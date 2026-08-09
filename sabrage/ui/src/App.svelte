<script lang="ts">
  import Sidebar from "./components/Sidebar.svelte";
  import About from "./screens/About.svelte";
  import Library from "./screens/Library.svelte";
  import Session from "./screens/Session.svelte";
  import Doctor from "./screens/Doctor.svelte";
  import Logs from "./screens/Logs.svelte";
  import Settings from "./screens/Settings.svelte";
  import type { Screen } from "./types";

  let screen = $state<Screen>("about");

  function navigate(next: Screen) {
    screen = next;
  }
</script>

<div class="app-shell">
  <Sidebar {screen} onNavigate={navigate} />

  <main class="main-area">
    {#if screen === "about"}
      <About onNavigate={navigate} />
    {:else if screen === "library"}
      <Library />
    {:else if screen === "session"}
      <Session />
    {:else if screen === "doctor"}
      <Doctor />
    {:else if screen === "logs"}
      <Logs />
    {:else if screen === "settings"}
      <Settings />
    {/if}
  </main>
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
