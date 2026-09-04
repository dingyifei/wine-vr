<script lang="ts">
  // The saved-games table — lists `libraryStore.rows`, expands one row at a
  // time into an install/bottle/patches/last-session detail panel, and drives
  // Run exactly like Session.svelte's `doLaunch` (same `sessionStore.launch`
  // + `stageStore.openGate` pair) so GateModal and the Session screen behave
  // identically regardless of which screen started the launch. Structure and
  // classes follow the mockup (Sabrage.dc.html lines 79-130) and the
  // conventions in Doctor.svelte/Session.svelte.

  import { onMount } from "svelte";
  import { errMsg } from "../lib/text";
  import { isLivePhase, type GameEntry, type GameStatus, type GameValidity, type GoldbergState, type LaunchOpts } from "../ipc";
  import { libraryStore } from "../stores/library.svelte";
  import { sessionStore } from "../stores/session.svelte";
  import { settingsStore } from "../stores/settings.svelte";
  import { stageStore } from "../stores/stage.svelte";
  import type { Screen } from "../types";

  interface Props {
    onNavigate?: (screen: Screen) => void;
    onEdit: (id: string) => void;
    onAdd: () => void;
  }
  let { onNavigate, onEdit, onAdd }: Props = $props();

  onMount(() => {
    void libraryStore.refresh();
    if (!settingsStore.loaded) void settingsStore.load();
  });

  // Goldberg is staged during run, well before `sessionStore.launch` settles
  // (that promise resolves only when the session ends), so a row's
  // `validity.goldberg`/status would stay at their pre-launch snapshot for the
  // whole session. `launchedAt` (session.svelte.ts) is the launch-local
  // "game is up" signal, set after Goldberg staging; this refresh does not
  // replace `runGame`'s settlement refresh, which updates `lastSession`.
  let lastRefreshedLaunchAt = $state<number | null>(null);
  $effect(() => {
    const at = sessionStore.launchedAt;
    if (at !== null && at !== lastRefreshedLaunchAt) {
      lastRefreshedLaunchAt = at;
      if (!libraryStore.loading) void libraryStore.refresh();
    }
  });

  const STATUS_LABEL: Record<GameStatus, string> = {
    ready: "Ready",
    needsAttention: "Needs attention",
    notFound: "Not found",
    needsSetup: "Needs setup",
  };
  const STATUS_CLASS: Record<GameStatus, string> = {
    ready: "tag-accent",
    needsAttention: "tag-outline",
    notFound: "tag-neutral",
    needsSetup: "tag-outline",
  };
  const GOLDBERG_LABEL: Record<GoldbergState, string> = {
    applied: "applied",
    appliedUnverified: "applied — no .orig-steam backup on this machine",
    original: "original steam_api64.dll still present — applied at next launch",
    modified: "unrecognized dll — reapplied at next launch",
    noDll: "no steam_api64.dll found yet",
  };


  let expandedId = $state<string | null>(null);

  function toggleRow(id: string) {
    expandedId = expandedId === id ? null : id;
  }

  // `effectiveLaunchOpts` mirrors `store::library::effective_options` on the
  // Rust side (per field, `override ?? global default`); the client, not the
  // backend, is the source of truth for the `LaunchOpts` flags sent over IPC.

  // `isLivePhase` excludes `"exited"`: the backend leaves that phase published
  // until the next launch, so a bare `phase !== "idle"` would hold every Run
  // button disabled for the rest of the app's life after one session ended.
  //
  // `!settingsStore.loadOk` holds Run disabled until settings.json loads: the
  // `?? false` fallback in `effectiveLaunchOpts` is right only for the backend's
  // "no settings.json yet" default — a corrupt file would launch wrong flags.
  //
  // `stageStore.running` covers a Hidden stage (the guard StagesPanel and
  // Doctor carry too): Hide clears `gate`, not the `runStage` behind it, and
  // GateModal won't adopt a second `openGate` — nothing is left to cancel it.
  const busy = $derived(
    sessionStore.launching ||
      isLivePhase(sessionStore.status.phase) ||
      stageStore.gate !== null ||
      stageStore.running ||
      !settingsStore.loadOk,
  );

  function effectiveLaunchOpts(entry: GameEntry): LaunchOpts {
    const global = settingsStore.settings?.launch;
    return {
      bottle: entry.bottle || null,
      bsDir: entry.bsDir || null,
      noAudio: entry.launchOverrides.noAudio ?? global?.noAudio ?? false,
      noDashboard: entry.launchOverrides.noDashboard ?? global?.noDashboard ?? false,
      wired: entry.launchOverrides.wired ?? global?.wired ?? false,
      verbose: entry.launchOverrides.verbose ?? global?.verbose ?? false,
      dryRun: false,
      gameId: entry.id,
    };
  }

  function runGame(entry: GameEntry) {
    const opts = effectiveLaunchOpts(entry);
    // Both calls fire together, same as Session.svelte's `doLaunch`: the store
    // owns the launch (and the rows GateModal reads), the gate is this launch's
    // window and renders any failure.
    // `launch` settles only after the backend records this run's `lastSession`
    // (when there is one), so refreshing on either settlement is the earliest
    // the row can show the current run in Last session.
    void sessionStore.launch(opts).then(
      () => void libraryStore.refresh(),
      () => void libraryStore.refresh(),
    );
    stageStore.openGate({ stage: "run", bottle: opts.bottle, bsDir: opts.bsDir, dryRun: false, launch: opts });
  }

  function isRunningFor(entry: GameEntry): boolean {
    // `"exited"` stays published until the next launch, so a bare `!== "idle"`
    // here would leave every row sharing the bottle reading "Running" long
    // after the game closed.
    if (!isLivePhase(sessionStore.status.phase)) return false;
    // A session this process launched remembers its Library entry, so two
    // entries sharing a bottle don't both read "Running". `SessionStatus`
    // carries no gameId, so a session Sabrage did not start (external or
    // re-attached) falls back to the bottle match.
    const launched = sessionStore.launchedGameId;
    if (launched !== null) return launched === entry.id;
    return sessionStore.status.bottle === entry.bottle;
  }

  function lastSessionCell(entry: GameEntry): string {
    if (isRunningFor(entry)) return "Running";
    if (!entry.lastSession) return "—";
    return new Date(entry.lastSession.startedAtUnixMs).toLocaleString();
  }

  function logBasename(path: string | null): string | null {
    if (!path) return null;
    const parts = path.split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function lastSessionDetail(entry: GameEntry): string {
    if (!entry.lastSession) return "No sessions yet";
    const exit = entry.lastSession.exitCode;
    const base = logBasename(entry.lastSession.logPath);
    return `exit ${exit ?? "—"}${base ? ` · ${base}` : ""}`;
  }

  function bottleDetail(entry: GameEntry, validity: GameValidity): string {
    const template = validity.bottleTemplate ?? "unknown template";
    const backend = validity.bottleBackendDxmt ? "DXMT" : "not DXMT (forced at launch)";
    return `${entry.bottle || "—"} · ${template} · ${backend}`;
  }


  let removeConfirmId = $state<string | null>(null);
  let removing = $state(false);
  let removeError = $state<string | null>(null);

  function requestRemove(id: string) {
    removeConfirmId = id;
    removeError = null;
  }
  function cancelRemove() {
    removeConfirmId = null;
  }
  async function confirmRemove(id: string) {
    removing = true;
    removeError = null;
    try {
      await libraryStore.remove(id);
      if (expandedId === id) expandedId = null;
    } catch (e) {
      removeError = errMsg(e);
    } finally {
      removing = false;
      removeConfirmId = null;
    }
  }
</script>

<div class="screen-header">
  <h3>Library</h3>
  <button class="btn btn-primary" onclick={onAdd}>
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"
      ><line x1="12" y1="5" x2="12" y2="19"></line><line x1="5" y1="12" x2="19" y2="12"></line></svg
    >
    Add game
  </button>
</div>

<div class="screen-body">
  {#if settingsStore.loaded && !settingsStore.loadOk}
    <div class="blueprint error-card settings-error">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <h6>Could not load settings</h6>
      <p class="text-muted">{settingsStore.error}</p>
      <p class="text-muted">
        Run is disabled until settings load successfully — a launch right now would silently fall back to
        default audio/dashboard/wired/verbose behavior instead of what's actually configured.
      </p>
      <button class="btn btn-secondary" onclick={() => void settingsStore.load()}>Retry</button>
    </div>
  {/if}
  {#if stageStore.running}
    <p class="text-muted">A stage is already running — wait for it to finish.</p>
  {/if}
  {#if libraryStore.loading && libraryStore.rows.length === 0}
    <p class="text-muted">Loading library…</p>
  {:else if libraryStore.error}
    <div class="blueprint error-card">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <h6>Could not load the library</h6>
      <p class="text-muted">{libraryStore.error}</p>
    </div>
  {:else if libraryStore.rows.length === 0}
    <div class="empty-state">
      <h5>No games yet</h5>
      <p class="text-muted">Add Beat Saber (or another Windows x64 game) to run it through the bridge.</p>
      <button class="btn btn-primary" onclick={onAdd}>Add game</button>
      <p class="text-muted depot-hint">
        Downloads the pinned depot with DepotDownloader — e.g. Beat Saber 1.29.4, the last build before the Meta
        account gate and the first with native OpenXR.
      </p>
    </div>
  {:else}
    <div class="table-head">
      <span class="col-game">Game</span>
      <span>Version</span>
      <span>Bottle</span>
      <span>Status</span>
      <span>Last session</span>
      <span></span>
    </div>
    <div class="rows">
      {#each libraryStore.rows as row (row.entry.id)}
        {@const entry = row.entry}
        {@const validity = row.validity}
        {@const open = expandedId === entry.id}
        <div class="game-block">
          <div
            class="game-row"
            class:open
            role="button"
            tabindex="0"
            onclick={() => toggleRow(entry.id)}
            onkeydown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                toggleRow(entry.id);
              }
            }}
          >
            <span class="col-game">
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-neutral-600)"
                stroke-width="1.5"
                class="chevron"
                class:open
                ><polyline points="9 18 15 12 9 6"></polyline></svg
              >
              <span class="game-name-block">
                <span class="game-name">{entry.name}</span>
                <span class="text-muted game-sub">App ID {entry.appid}</span>
              </span>
            </span>
            <span>{validity.detectedVersion ?? "—"}</span>
            <span>{entry.bottle || "—"}</span>
            <span><span class="tag {STATUS_CLASS[validity.status]}">{STATUS_LABEL[validity.status]}</span></span>
            <span class="text-muted last-cell">{lastSessionCell(entry)}</span>
            <span class="run-cell">
              <button
                class="btn btn-primary run-btn"
                disabled={busy}
                onclick={(e) => {
                  e.stopPropagation();
                  runGame(entry);
                }}
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor" stroke="none"
                  ><polygon points="6 3 20 12 6 21 6 3"></polygon></svg
                >
                Run
              </button>
            </span>
          </div>

          {#if open}
            <div class="detail-panel">
              <div class="detail-grid">
                <div>
                  <div class="detail-label">Install dir</div>
                  <div class="detail-mono">{entry.bsDir || "—"}</div>
                </div>
                <div>
                  <div class="detail-label">Bottle</div>
                  <div>{bottleDetail(entry, validity)}</div>
                </div>
                <div>
                  <div class="detail-label">Patches</div>
                  <div>Goldberg steam_api64.dll — applied at every launch ({GOLDBERG_LABEL[validity.goldberg]})</div>
                </div>
                <div>
                  <div class="detail-label">Last session</div>
                  <div>{lastSessionDetail(entry)}</div>
                </div>
              </div>

              {#if validity.problems.length > 0}
                <div class="problems">
                  {#each validity.problems as problem, i (i)}
                    <div class="problem-line">{problem}</div>
                  {/each}
                </div>
              {/if}

              <div class="detail-actions">
                <button class="btn btn-primary" disabled={busy} onclick={() => runGame(entry)}
                  >Run through bridge</button
                >
                <button class="btn btn-secondary" onclick={() => onEdit(entry.id)}>Edit configuration…</button>
                <button class="btn btn-ghost" onclick={() => onNavigate?.("doctor")}>Doctor</button>

                {#if removeConfirmId === entry.id}
                  <span class="remove-confirm">
                    <span class="text-muted">Remove this game from the library?</span>
                    <button class="btn btn-secondary" onclick={cancelRemove} disabled={removing}>Cancel</button>
                    <button class="btn btn-primary danger-btn" onclick={() => confirmRemove(entry.id)} disabled={removing}>
                      {removing ? "Removing…" : "Yes, remove"}
                    </button>
                  </span>
                {:else}
                  <button class="btn btn-ghost remove-btn" onclick={() => requestRemove(entry.id)}>Remove</button>
                {/if}
              </div>
              {#if removeError && removeConfirmId === null}
                <div class="remove-error">Remove failed: {removeError}</div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
    <p class="text-muted footer-note">
      Run resets the bottle's wineserver, preflights the bridge, applies the Goldberg emulator and routes audio —
      then launches through DXMT → oxrsys → ALVR.
    </p>
  {/if}
</div>

<style>
  .screen-header {
    padding: 22px 28px 14px;
    border-bottom: 1px solid var(--color-divider);
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
  }
  .screen-header h3 {
    margin: 0;
  }
  .screen-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 20px 28px 28px;
  }
  .error-card {
    padding: 18px 20px;
    max-width: 460px;
  }
  .error-card h6 {
    color: var(--color-accent-900);
    margin-bottom: 6px;
  }
  .error-card p {
    font-size: 13px;
    margin: 0;
  }
  .error-card p + p {
    margin-top: 6px;
  }
  .settings-error {
    margin-bottom: 16px;
  }
  .settings-error .btn {
    margin-top: 10px;
  }
  .empty-state {
    max-width: 460px;
    padding: 8px 0;
  }
  .empty-state h5 {
    margin-bottom: 6px;
  }
  .depot-hint {
    font-size: 11.5px;
    margin-top: 14px;
  }
  .table-head,
  .game-row {
    display: grid;
    grid-template-columns: 2.6fr 0.8fr 0.9fr 1.1fr 1.6fr 0.9fr;
    gap: 0 12px;
    align-items: center;
  }
  .table-head {
    padding: 0 10px 7px;
    border-bottom: 1px solid var(--color-divider);
  }
  .table-head span {
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-text) 60%, transparent);
  }
  .rows {
    display: flex;
    flex-direction: column;
  }
  .game-block {
    border-bottom: 1px solid color-mix(in srgb, var(--color-text) 8%, transparent);
  }
  .game-row {
    width: 100%;
    padding: 9px 10px;
    font-size: 14px;
    text-align: left;
    cursor: pointer;
    background: transparent;
    border: none;
    font-family: inherit;
    color: inherit;
  }
  .game-row:hover,
  .game-row.open {
    background: color-mix(in srgb, var(--color-text) 4%, transparent);
  }
  .col-game {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }
  .chevron {
    flex: none;
    transition: transform 0.15s;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .game-name-block {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .game-name {
    font-weight: 500;
  }
  .game-sub {
    font-size: 11.5px;
  }
  .last-cell {
    font-size: 12.5px;
  }
  .run-cell {
    text-align: right;
    white-space: nowrap;
  }
  .run-btn {
    padding: 4px 14px;
    font-size: 13px;
  }
  .detail-panel {
    padding: 14px 10px 16px 31px;
    background: color-mix(in srgb, var(--color-accent) 5%, transparent);
    border-top: 1px dashed var(--color-divider);
  }
  .detail-grid {
    display: grid;
    grid-template-columns: 1.4fr 1fr 1fr 1.3fr;
    gap: 18px;
    font-size: 12.5px;
  }
  .detail-label {
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-text) 55%, transparent);
    margin-bottom: 2px;
  }
  .detail-mono {
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11px;
    word-break: break-all;
  }
  .problems {
    margin-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .problem-line {
    font-size: 12px;
    color: var(--color-accent-900);
  }
  .detail-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 13px;
    flex-wrap: wrap;
  }
  .detail-actions .btn {
    padding: 4px 16px;
    font-size: 13px;
  }
  .remove-btn {
    margin-left: auto;
  }
  .danger-btn {
    background: var(--color-accent-900);
    border-color: var(--color-accent-900);
    color: var(--color-bg);
  }
  .danger-btn:hover {
    background: color-mix(in srgb, var(--color-accent-900) 85%, black);
  }
  .remove-confirm {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
    font-size: 12.5px;
  }
  .remove-error {
    font-size: 11.5px;
    color: var(--color-accent-900);
    margin-top: 6px;
  }
  .footer-note {
    font-size: 12px;
    margin-top: 14px;
  }
</style>
