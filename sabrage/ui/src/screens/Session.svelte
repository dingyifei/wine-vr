<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getAppState, type LaunchOpts, type SessionStatus, type StageEvent } from "../ipc";
  import { sessionStore } from "../stores/session.svelte";
  import { stageStore } from "../stores/stage.svelte";
  import type { Screen } from "../types";

  interface Props {
    onNavigate?: (screen: Screen) => void;
  }
  let { onNavigate }: Props = $props();

  // ── bottle + launch options ─────────────────────────────────────────────────

  let bottles = $state<string[]>([]);
  let bottlesLoaded = $state(false);
  let selectedBottle = $state("");
  let bsDir = $state("");
  let noAudio = $state(false);
  let noDashboard = $state(false);
  let wired = $state(false);
  let verbose = $state(false);
  let copied = $state(false);

  function pickDefaultBottle(list: string[]): string {
    return list.includes("Steam") ? "Steam" : (list[0] ?? "");
  }

  onMount(async () => {
    try {
      const state = await getAppState();
      bottles = state.bottles;
      selectedBottle = pickDefaultBottle(bottles);
    } catch {
      bottles = [];
    } finally {
      bottlesLoaded = true;
    }
    try {
      // "Previous session did not shut down cleanly" / "running outside this
      // instance" banner — one pass at mount, against whichever bottle is
      // selected once bottles have loaded (falls back to none selected on an
      // empty bottle list, which is still a meaningful reconcile: any
      // recorded session belongs to *some* bottle regardless).
      await sessionStore.reconcile(selectedBottle || null);
    } catch {
      // best-effort — a failed reconcile just means no banner, not a blocked screen
    }
  });

  // 1 Hz uptime tick.
  let nowMs = $state(Date.now());
  let tickHandle: ReturnType<typeof setInterval> | null = null;
  onMount(() => {
    tickHandle = setInterval(() => {
      nowMs = Date.now();
    }, 1000);
  });
  onDestroy(() => {
    if (tickHandle) clearInterval(tickHandle);
  });

  const status = $derived(sessionStore.status);

  const busy = $derived(
    sessionStore.launching ||
      status.phase === "running" ||
      status.phase === "launching" ||
      status.phase === "preflight" ||
      stageStore.gate !== null,
  );

  function equivalentCommand(): string {
    const parts = ["./demo.sh", "run", "--bottle", selectedBottle || "<name>"];
    if (bsDir.trim()) parts.push("--bs-dir", `"${bsDir.trim()}"`);
    if (noAudio) parts.push("--no-audio");
    if (noDashboard) parts.push("--no-dashboard");
    if (wired) parts.push("--wired");
    if (verbose) parts.push("--verbose");
    return parts.join(" ");
  }

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(equivalentCommand());
      copied = true;
      setTimeout(() => {
        copied = false;
      }, 1500);
    } catch {
      // Clipboard access can be denied in some webview contexts; the command
      // is still visible to select and copy by hand.
    }
  }

  function doLaunch(dryRun: boolean) {
    const opts: LaunchOpts = {
      bottle: selectedBottle || null,
      bsDir: bsDir.trim() ? bsDir.trim() : null,
      noAudio,
      noDashboard,
      wired,
      verbose,
      dryRun,
    };
    // Both calls fire together: the store owns the actual launch (and the
    // rows GateModal reads), the gate is just this same launch's window.
    void sessionStore.launch(opts);
    stageStore.openGate({
      stage: "run",
      bottle: opts.bottle,
      bsDir: opts.bsDir,
      dryRun,
      launch: opts,
    });
  }

  // ── reconcile banner ─────────────────────────────────────────────────────────
  // Derived from already-documented sessionStore surface only
  // (`status` + `reconcileRows`) rather than a bespoke "what kind of
  // reconcile was this" field — see the report's NEEDS FROM note if a richer
  // signal becomes available.

  let bannerDismissed = $state(false);
  const reconcileBanner = $derived.by(() => {
    if (bannerDismissed) return null;
    if (sessionStore.reconcileRows.length > 0) {
      return `Previous session did not shut down cleanly — restored: ${sessionStore.reconcileRows.join(", ")}`;
    }
    if (status.phase !== "idle" && !status.ownedByThisProcess) {
      return `A session is running outside this Sabrage instance (pid ${status.pid ?? "—"}) — Stop or re-attach.`;
    }
    return null;
  });

  // ── stop ─────────────────────────────────────────────────────────────────────

  let stopping = $state(false);
  let stopError = $state<string | null>(null);
  /** The stop stage's own lines, shown as a small local list next to the
   * button — scoped to this component (unlike `launchRows`, nothing else
   * needs to read a Stop invocation's progress) so it rides the same
   * `(ev) => void` callback `applyFix`/`runStage` already take. Only ever
   * populated for a session this process does *not* supervise (`stop_session`
   * runs the real `Stop` stage there, which does emit rows on `onEvent`) —
   * see `sessionLogTail` below for the supervised case. */
  let stopRows = $state<string[]>([]);

  function stopRowText(ev: StageEvent): string | null {
    switch (ev.kind) {
      case "line":
      case "text":
        return ev.text;
      case "fatal":
        return ev.message;
      default:
        return null;
    }
  }

  /** How many trailing lines of `sessionLogTail` to show. */
  const SESSION_LOG_TAIL_LINES = 12;

  /** Teardown rows for a session this process launched land on the *launch*
   * invocation's own event channel (`sessionStore.launchRows`), never on the
   * Stop button's local `onEvent` callback — `commands::stop_session`'s own
   * doc comment: stopping a session this process supervises fires its cancel
   * token and streams "nothing new" on `on_event`, because the still-pending
   * `launch()` call is what emits every resulting row itself, onto
   * `launchRows`. Without this, Stop showed "Stopping…" with no rows at all
   * for the one case a user watching Stop actually cares about. */
  const sessionLogTail = $derived.by((): string[] => {
    if (!status.ownedByThisProcess) return [];
    const lines: string[] = [];
    for (const ev of sessionStore.launchRows) {
      const text = stopRowText(ev);
      if (text != null) lines.push(text);
    }
    return lines.slice(-SESSION_LOG_TAIL_LINES);
  });

  async function doStop() {
    stopping = true;
    stopError = null;
    stopRows = [];
    try {
      await sessionStore.stop((ev) => {
        const text = stopRowText(ev);
        if (text != null) stopRows.push(text);
      });
    } catch (e) {
      stopError = e instanceof Error ? e.message : String(e);
    } finally {
      stopping = false;
    }
  }

  // ── detach ───────────────────────────────────────────────────────────────────

  let detaching = $state(false);
  let detachError = $state<string | null>(null);

  const canDetach = $derived(status.ownedByThisProcess && status.phase === "running");

  async function doDetach() {
    detaching = true;
    detachError = null;
    try {
      await sessionStore.detach();
    } catch (e) {
      detachError = e instanceof Error ? e.message : String(e);
    } finally {
      detaching = false;
    }
  }

  // ── display helpers ──────────────────────────────────────────────────────────

  const PHASE_LABEL: Record<SessionStatus["phase"], string> = {
    idle: "Idle",
    preflight: "Preflight",
    launching: "Launching",
    running: "Running",
    stalled: "Stalled",
    stopping: "Stopping",
    exited: "Exited",
    detached: "Detached",
  };
  const PHASE_CLASS: Record<SessionStatus["phase"], string> = {
    idle: "phase-idle",
    preflight: "phase-amber",
    launching: "phase-amber",
    running: "phase-accent",
    stalled: "phase-alert",
    stopping: "phase-amber",
    exited: "phase-idle",
    detached: "phase-idle",
  };

  function formatUptime(ms: number): string {
    const total = Math.max(0, Math.floor(ms / 1000));
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    const pad = (n: number) => n.toString().padStart(2, "0");
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
  }

  const uptimeText = $derived.by(() => {
    if (status.startedAtUnixMs == null) return "—";
    return formatUptime(nowMs - status.startedAtUnixMs);
  });

  function encoderChip(s: SessionStatus): string {
    const e = s.encoder;
    if (!e) return "waiting for encoder…";
    return `${e.codec} · ${e.path} · ${e.width}x${e.height} @${e.refreshHz}Hz · ${e.bitrateMbps} Mbps`;
  }

  const hasSession = $derived(status.phase !== "idle");
  const canStop = $derived(
    status.phase === "running" ||
      status.phase === "stalled" ||
      status.phase === "launching" ||
      status.phase === "preflight" ||
      status.phase === "detached",
  );
</script>

<div class="screen-header">
  <h3>Session</h3>
</div>

<div class="screen-body">
  {#if reconcileBanner}
    <div class="banner">
      <span>{reconcileBanner}</span>
      <button class="btn btn-ghost banner-dismiss" onclick={() => (bannerDismissed = true)}>Dismiss</button>
    </div>
  {/if}

  <div class="cards">
    <div class="blueprint card-panel launch-card">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <div class="card-kicker">Launch</div>
      <h5 class="card-panel-title">Beat Saber through the bridge</h5>

      <div class="field">
        <label for="session-bottle">Bottle</label>
        {#if bottlesLoaded && bottles.length === 0}
          <span class="text-muted">none found — create one in the CrossOver UI</span>
        {:else}
          <select id="session-bottle" class="input" bind:value={selectedBottle} disabled={busy}>
            {#each bottles as b (b)}
              <option value={b}>{b}</option>
            {/each}
          </select>
        {/if}
      </div>

      <div class="field">
        <label for="session-bsdir">Beat Saber directory</label>
        <input
          id="session-bsdir"
          class="input"
          type="text"
          placeholder="leave empty to derive from the bottle (env: WINEVR_BS_DIR)"
          bind:value={bsDir}
          disabled={busy}
        />
      </div>

      <div class="toggles">
        <label class="toggle-row">
          <input type="checkbox" bind:checked={noAudio} disabled={busy} />
          <span class="toggle-body">
            <span class="toggle-title">No audio routing</span>
            <span class="text-muted toggle-desc">Keep game audio on the Mac instead of routing it to BlackHole.</span>
          </span>
        </label>
        <label class="toggle-row">
          <input type="checkbox" bind:checked={noDashboard} disabled={busy} />
          <span class="toggle-body">
            <span class="toggle-title">No dashboard</span>
            <span class="text-muted toggle-desc">Don't open the ALVR server dashboard window.</span>
          </span>
        </label>
        <label class="toggle-row">
          <input type="checkbox" bind:checked={wired} disabled={busy} />
          <span class="toggle-body">
            <span class="toggle-title">Wired (USB)</span>
            <span class="text-muted toggle-desc">
              Forward the stream ports over adb for a wired Quest instead of WiFi.
            </span>
          </span>
        </label>
        <label class="toggle-row">
          <input type="checkbox" bind:checked={verbose} disabled={busy} />
          <span class="toggle-body">
            <span class="toggle-title">Verbose wine log</span>
            <span class="text-muted toggle-desc">
              Keep the full wine/OpenXR debug firehose instead of the quiet default.
            </span>
          </span>
        </label>
      </div>

      <div class="cmd-row">
        <code class="cmd-text">{equivalentCommand()}</code>
        <button class="btn btn-ghost cmd-copy-btn" onclick={copyCommand}>{copied ? "Copied" : "Copy"}</button>
      </div>

      <div class="launch-actions">
        <button class="btn btn-secondary" disabled={busy} onclick={() => doLaunch(true)}>Dry-run</button>
        <button class="btn btn-primary" disabled={busy} onclick={() => doLaunch(false)}>Launch</button>
      </div>
    </div>

    <div class="blueprint card-panel status-card">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <div class="card-kicker">Status</div>

      <div class="phase-row">
        <span class="phase-dot {PHASE_CLASS[status.phase]}"></span>
        <span class="phase-label">{PHASE_LABEL[status.phase]}</span>
        {#if status.exitCode != null}
          <span class="text-muted exit-note">exit {status.exitCode}</span>
        {/if}
      </div>

      {#if !hasSession}
        <p class="text-muted no-session">No session running. Use the Launch card to start one.</p>
      {:else}
        <div class="stats">
          <div class="stat-row">
            <span class="stat-label">Bottle</span>
            <span class="stat-value">{status.bottle ?? "—"}</span>
          </div>
          <div class="stat-row">
            <span class="stat-label">PID</span>
            <span class="stat-value">{status.pid ?? "—"}</span>
          </div>
          <div class="stat-row">
            <span class="stat-label">Uptime</span>
            <span class="stat-value">{uptimeText}</span>
          </div>
          <div class="stat-row">
            <span class="stat-label">Encoder</span>
            <span class="stat-value">{encoderChip(status)}</span>
          </div>
          <div class="stat-row">
            <span class="stat-label">Runtime state</span>
            <span class="stat-value">
              {status.runtimeState ?? "—"}
              {#if status.runtimeState && !status.runtimeFresh}
                <span class="text-muted stale-note">(stale)</span>
              {/if}
            </span>
          </div>
          <div class="stat-row">
            <span class="stat-label">Log</span>
            <span class="stat-value log-value">
              <span class="log-path">{status.logPath ?? "—"}</span>
              {#if status.logPath}
                <button class="btn btn-ghost open-logs-btn" onclick={() => onNavigate?.("logs")}>
                  Open in Logs
                </button>
              {/if}
            </span>
          </div>
        </div>

        {#if status.phase === "stalled"}
          <div class="stalled-banner">Stream stalled — wake the headset.</div>
        {/if}

        <div class="status-actions">
          {#if canStop}
            <button class="btn btn-primary danger-btn" onclick={doStop} disabled={stopping}>
              {stopping ? "Stopping…" : "Stop"}
            </button>
          {/if}
          {#if canDetach}
            <button class="btn btn-secondary" onclick={doDetach} disabled={detaching}>
              {detaching ? "Detaching…" : "Detach"}
            </button>
          {/if}
          {#if status.detached && !status.ownedByThisProcess}
            <button class="btn btn-secondary" onclick={() => onNavigate?.("logs")}>Re-attach (follow log)</button>
          {/if}
        </div>
        {#if canDetach}
          <p class="text-muted detach-note">
            Stop supervising — the game keeps streaming; audio is restored on the next Sabrage launch or Stop.
          </p>
        {/if}
        {#if status.ownedByThisProcess}
          {#if (stopping || status.phase === "stopping" || status.phase === "exited") && sessionLogTail.length > 0}
            <div class="stop-rows">
              <div class="stop-rows-label text-muted">Session log</div>
              {#each sessionLogTail as line, i (i)}
                <div class="stop-row-line">{line}</div>
              {/each}
            </div>
          {/if}
        {:else if stopRows.length > 0}
          <div class="stop-rows">
            {#each stopRows as line, i (i)}
              <div class="stop-row-line">{line}</div>
            {/each}
          </div>
        {/if}
        {#if stopError}
          <div class="text-muted stop-error">Stop failed: {stopError}</div>
        {/if}
        {#if detachError}
          <div class="text-muted stop-error">Detach failed: {detachError}</div>
        {/if}
      {/if}
    </div>
  </div>
</div>

<style>
  .screen-header {
    padding: 22px 28px 14px;
    border-bottom: 1px solid var(--color-divider);
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
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    border: 1px solid var(--color-divider);
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
    padding: 8px 14px;
    font-size: 12.5px;
    margin-bottom: 16px;
  }
  .banner-dismiss {
    flex: none;
    font-size: 11.5px;
    padding: 2px 8px;
  }
  .cards {
    display: grid;
    grid-template-columns: minmax(280px, 1fr) minmax(280px, 1fr);
    gap: 20px;
    align-items: start;
  }
  @media (max-width: 860px) {
    .cards {
      grid-template-columns: 1fr;
    }
  }
  .card-panel {
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .card-panel-title {
    margin: 0;
  }
  .toggles {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .toggle-row {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    cursor: pointer;
  }
  .toggle-row input {
    margin-top: 3px;
    width: 14px;
    height: 14px;
    accent-color: var(--color-accent);
    flex: none;
  }
  .toggle-body {
    display: flex;
    flex-direction: column;
  }
  .toggle-title {
    font-size: 13px;
  }
  .toggle-desc {
    font-size: 11.5px;
  }
  .cmd-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cmd-text {
    flex: 1;
    min-width: 0;
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11.5px;
    background: var(--color-surface);
    border: 1px solid var(--color-divider);
    padding: 5px 8px;
    overflow-x: auto;
    white-space: pre;
  }
  .cmd-copy-btn {
    flex: none;
    font-size: 11.5px;
    padding: 2px 8px;
  }
  .launch-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 2px;
  }
  .phase-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .phase-dot {
    width: 9px;
    height: 9px;
    flex: none;
    border-radius: 50%;
  }
  .phase-idle {
    background: var(--color-neutral-400);
  }
  .phase-amber {
    background: #b8862f;
  }
  .phase-accent {
    background: var(--color-accent);
  }
  .phase-alert {
    background: var(--color-accent-900);
  }
  .phase-label {
    font-family: var(--font-heading);
    font-weight: 600;
    font-size: 15px;
  }
  .exit-note {
    font-size: 11.5px;
  }
  .no-session {
    font-size: 13px;
    margin: 4px 0 0;
  }
  .stats {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .stat-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-size: 13px;
  }
  .stat-label {
    flex: none;
    width: 92px;
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-text) 55%, transparent);
  }
  .stat-value {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .log-value {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .log-path {
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11.5px;
  }
  .open-logs-btn {
    flex: none;
    font-size: 11.5px;
    padding: 1px 6px;
  }
  .stale-note {
    font-size: 11px;
  }
  .stalled-banner {
    border: 1px solid var(--color-divider);
    background: color-mix(in srgb, var(--color-accent-900) 12%, transparent);
    padding: 7px 10px;
    font-size: 12.5px;
  }
  .status-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .danger-btn {
    background: var(--color-accent-900);
    border-color: var(--color-accent-900);
    color: var(--color-bg);
  }
  .danger-btn:hover {
    background: color-mix(in srgb, var(--color-accent-900) 85%, black);
  }
  .stop-rows {
    border: 1px solid var(--color-divider);
    background: var(--color-surface);
    padding: 6px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 120px;
    overflow-y: auto;
  }
  .stop-rows-label {
    font-size: 10.5px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    margin-bottom: 2px;
  }
  .detach-note {
    font-size: 11.5px;
    margin: -2px 0 0;
  }
  .stop-row-line {
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11.5px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .stop-error {
    font-size: 11.5px;
    color: var(--color-accent-900);
  }
</style>
