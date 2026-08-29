<script lang="ts">
  import NavItem from "./NavItem.svelte";
  import { stageStore } from "../stores/stage.svelte";
  import type { Screen } from "../types";

  interface Props {
    screen: Screen;
    onNavigate: (screen: Screen) => void;
    /** Sidebar nav dot on Doctor — wired to doctor://badge once Doctor is real (Phase 1). Static false for now. */
    doctorBadge?: boolean;
  }

  let { screen, onNavigate, doctorBadge = false }: Props = $props();
</script>

<div class="sidebar">
  <div class="wordmark-band" data-tauri-drag-region>
    <div class="wordmark">SABRAGE</div>
  </div>

  <nav class="nav-list">
    <NavItem active={screen === "about"} onclick={() => onNavigate("about")}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="12" cy="12" r="9"></circle>
        <line x1="12" y1="16" x2="12" y2="12"></line>
        <line x1="12" y1="8" x2="12.01" y2="8"></line>
      </svg>
      About
    </NavItem>
    <NavItem active={screen === "library"} onclick={() => onNavigate("library")}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <line x1="8" y1="6" x2="21" y2="6"></line>
        <line x1="8" y1="12" x2="21" y2="12"></line>
        <line x1="8" y1="18" x2="21" y2="18"></line>
        <line x1="3" y1="6" x2="3.01" y2="6"></line>
        <line x1="3" y1="12" x2="3.01" y2="12"></line>
        <line x1="3" y1="18" x2="3.01" y2="18"></line>
      </svg>
      Library
    </NavItem>
    <NavItem active={screen === "session"} onclick={() => onNavigate("session")}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"></polyline>
      </svg>
      Session
    </NavItem>
    <NavItem active={screen === "doctor"} badge={doctorBadge} onclick={() => onNavigate("doctor")}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"></path>
        <polyline points="22 4 12 14.01 9 11.01"></polyline>
      </svg>
      Doctor
    </NavItem>
    <NavItem active={screen === "logs"} onclick={() => onNavigate("logs")}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <polyline points="4 17 10 11 4 5"></polyline>
        <line x1="12" y1="19" x2="20" y2="19"></line>
      </svg>
      Logs
    </NavItem>
    <NavItem active={screen === "settings"} onclick={() => onNavigate("settings")}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <line x1="21" y1="4" x2="14" y2="4"></line>
        <line x1="10" y1="4" x2="3" y2="4"></line>
        <line x1="21" y1="12" x2="12" y2="12"></line>
        <line x1="8" y1="12" x2="3" y2="12"></line>
        <line x1="21" y1="20" x2="16" y2="20"></line>
        <line x1="12" y1="20" x2="3" y2="20"></line>
        <line x1="14" y1="2" x2="14" y2="6"></line>
        <line x1="8" y1="10" x2="8" y2="14"></line>
        <line x1="16" y1="18" x2="16" y2="22"></line>
      </svg>
      Settings
    </NavItem>
  </nav>

  <div class="footer">
    <!-- Honest stubs: no backend exists yet — real state arrives with get_app_state() in Phase 1 -->
    <div class="status-row">
      <span class="status-dot"></span>
      No backend yet
    </div>
    <div class="text-muted bottle-line">Bottle · not detected yet</div>
    <button class="btn btn-ghost setup-btn" onclick={() => stageStore.openStagesPanel()}>Setup</button>
    <div class="text-muted version-line">BRIDGE — · ALVR v20.14.1</div>
  </div>
</div>

<style>
  .sidebar {
    width: 204px;
    flex: none;
    display: flex;
    flex-direction: column;
    height: 100%;
    border-right: 1px solid var(--color-divider);
  }
  .wordmark-band {
    /* ~52px top padding keeps the wordmark clear of the native traffic
       lights that float over this drag region under titleBarStyle:Overlay. */
    padding: 52px 16px 14px;
  }
  .wordmark {
    font-family: var(--font-heading);
    font-weight: 600;
    font-size: 22px;
    letter-spacing: 0.06em;
  }
  .nav-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 8px;
  }
  .footer {
    margin-top: auto;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-top: 1px solid var(--color-divider);
  }
  .status-row {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
  }
  .status-dot {
    width: 8px;
    height: 8px;
    flex: none;
    background: var(--color-neutral-400);
  }
  .bottle-line {
    font-size: 11px;
  }
  .setup-btn {
    justify-content: flex-start;
    font-size: 12.5px;
    padding: 2px 4px;
  }
  .version-line {
    font-size: 10px;
    letter-spacing: 0.06em;
  }
</style>
