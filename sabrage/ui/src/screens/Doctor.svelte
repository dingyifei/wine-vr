<script lang="ts">
  import { onMount } from "svelte";
  import CheckRow from "../components/CheckRow.svelte";
  import { doctorStore } from "../stores/doctor.svelte";
  import { stageStore } from "../stores/stage.svelte";
  import { applyFix, contractFixIdToAction, FIX_META, type FixAction } from "../ipc";

  interface FixError {
    slug: string;
    message: string;
    remedy: string | null;
    fix: FixAction | null;
  }

  let selectedBottle = $state("");
  let fixBusySlug = $state<string | null>(null);
  let fixError = $state<FixError | null>(null);
  let confirmFix = $state<{ slug: string; action: FixAction } | null>(null);

  function pickDefaultBottle(bottles: string[], preferred: string | null): string {
    if (preferred && bottles.includes(preferred)) return preferred;
    return bottles.includes("Steam") ? "Steam" : (bottles[0] ?? "");
  }

  onMount(async () => {
    await doctorStore.loadBottles();
    selectedBottle = pickDefaultBottle(doctorStore.bottles, doctorStore.defaultBottle);
    void runChecks();
  });

  async function runChecks() {
    await doctorStore.run({ bottle: selectedBottle || null });
  }

  /** Handles a CheckRow's Fix button. Whole-stage fixes (`run-setup` /
   * `run-build` / `run-install`) open the shared GateModal so the run streams
   * exactly like a Pipeline-panel invocation; the rest run in place via
   * `fix()` and re-run doctor afterward — a destructive one confirms first. */
  function handleFixRequest(slug: string, fixId: string) {
    const action = contractFixIdToAction(fixId);
    if (!action) return;
    dispatchFix(slug, action);
  }

  /** The part of `handleFixRequest` that only needs an already-resolved
   * `FixAction` — shared with the error banner's "try the suggested fix"
   * button, whose `FixAction` comes straight off a `Fatal` event rather than
   * a contract id string. */
  function dispatchFix(slug: string, action: FixAction) {
    const meta = FIX_META[action];
    if (meta.stage) {
      stageStore.openGate({
        stage: meta.stage,
        bottle: selectedBottle || null,
        onFinished: () => void runChecks(),
      });
      return;
    }
    if (meta.destructive) {
      confirmFix = { slug, action };
      return;
    }
    void runFix(slug, action);
  }

  function cap(s: string): string {
    return s.length ? s[0].toUpperCase() + s.slice(1) : s;
  }

  interface FatalInfo {
    message: string;
    remedy: string | null;
    fix: FixAction | null;
  }

  async function runFix(slug: string, action: FixAction) {
    fixBusySlug = slug;
    fixError = null;
    confirmFix = null;
    // A failing fix announces itself as a `Fatal` event on this same stream
    // (message + structured remedy + a possible follow-up fix id) before the
    // `applyFix` promise ever rejects — capture it so the error state below
    // can show the same detail the GateModal shows for a stage's own Fatal,
    // instead of only the rejected promise's bare message.
    let fatal: FatalInfo | null = null;
    try {
      await applyFix(action, { bottle: selectedBottle || null }, true, (ev) => {
        if (ev.kind === "fatal") {
          fatal = { message: ev.message, remedy: ev.remedy, fix: ev.fix };
        }
      });
    } catch (e) {
      // `fatal` is only ever reassigned inside the callback above, so
      // TypeScript's control-flow narrowing — which can't prove that
      // callback ran before this line — types every read of it here as the
      // initializer's literal `null`, not the declared union. The cast back
      // to the declared type is what the runtime already knows to be true;
      // the truthiness check below still does the actual branching.
      const captured = fatal as FatalInfo | null;
      fixError = captured
        ? { slug, message: captured.message, remedy: captured.remedy, fix: captured.fix }
        : { slug, message: e instanceof Error ? e.message : String(e), remedy: null, fix: null };
    } finally {
      fixBusySlug = null;
      void runChecks();
    }
  }

  /** "bottle-bridge" -> "Bottle Bridge" — the contract's `group` field, title-cased. */
  function titleCase(group: string): string {
    return group
      .split("-")
      .map((w) => (w.length ? w[0].toUpperCase() + w.slice(1) : w))
      .join(" ");
  }

  const summaryText = $derived.by(() => {
    const s = doctorStore.summary;
    if (!s) {
      const done = doctorStore.rows.filter((r) => r.phase === "done").length;
      return done > 0 ? `${done} checked so far` : "Running checks…";
    }
    if (s.failCount === 0 && s.warnCount === 0) return `All ${s.total} checks passed`;
    if (s.failCount === 0) {
      return `All ${s.total} checks passed · ${s.warnCount} warning${s.warnCount === 1 ? "" : "s"}`;
    }
    return `${s.failCount} check${s.failCount === 1 ? "" : "s"} failed, ${s.warnCount} warning${
      s.warnCount === 1 ? "" : "s"
    } — remedies inline`;
  });
</script>

<div class="screen-header">
  <div class="header-top">
    <h3>Doctor</h3>
    <div class="header-actions">
      <span class="text-muted summary-text">{summaryText}</span>
      <button class="btn btn-primary" onclick={runChecks} disabled={doctorStore.running}>
        {doctorStore.running ? "Running…" : "Run checks"}
      </button>
    </div>
  </div>
  <div class="bottle-row">
    <label class="text-muted" for="doctor-bottle">Bottle</label>
    {#if doctorStore.bottlesLoaded && doctorStore.bottles.length === 0}
      <span class="text-muted">none found — create one in the CrossOver UI</span>
    {:else}
      <select
        id="doctor-bottle"
        class="input bottle-select"
        bind:value={selectedBottle}
        disabled={doctorStore.running}
      >
        {#each doctorStore.bottles as b (b)}
          <option value={b}>{b}</option>
        {/each}
      </select>
    {/if}
  </div>
</div>

<div class="screen-body">
  {#if doctorStore.rows.length === 0}
    {#if doctorStore.error}
      <div class="blueprint error-card">
        <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
        <h6>Could not run doctor</h6>
        <p class="text-muted">{doctorStore.error}</p>
      </div>
    {:else if doctorStore.running}
      <p class="text-muted">Running checks…</p>
    {:else}
      <p class="text-muted">Doctor checks have not run yet.</p>
    {/if}
  {:else}
    <div class="rows">
      {#each doctorStore.rows as row, i (row.slug)}
        {#if i === 0 || row.group !== doctorStore.rows[i - 1].group}
          <h6 class="group-header">{titleCase(row.group)}</h6>
        {/if}
        <CheckRow
          {row}
          isRunning={row.slug === doctorStore.runningSlug}
          busy={fixBusySlug === row.slug}
          onFix={(fixId) => handleFixRequest(row.slug, fixId)}
        />
      {/each}
    </div>
  {/if}
</div>

{#if fixError}
  <div class="fix-error-banner">
    <div class="text-muted">Fix failed: {fixError.message}</div>
    {#if fixError.remedy}<div class="fix-error-remedy">{fixError.remedy}</div>{/if}
    {#if fixError.fix}
      <button
        class="btn btn-ghost fix-error-retry"
        onclick={() => dispatchFix(fixError!.slug, fixError!.fix!)}
      >
        {FIX_META[fixError.fix].stage ? `Open ${cap(FIX_META[fixError.fix].stage!)}` : "Try suggested fix"}
      </button>
    {/if}
  </div>
{/if}

{#if confirmFix}
  <div class="confirm-backdrop">
    <div class="dialog blueprint elev-md confirm-dialog">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <div class="dialog-title">{FIX_META[confirmFix.action].title}</div>
      <p class="dialog-body">This cannot be undone. Continue?</p>
      <div class="dialog-actions">
        <button class="btn btn-secondary" onclick={() => (confirmFix = null)}>Cancel</button>
        <button class="btn btn-primary" onclick={() => runFix(confirmFix!.slug, confirmFix!.action)}>
          Yes, continue
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .screen-header {
    padding: 18px 28px 14px;
    border-bottom: 1px solid var(--color-divider);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .header-top {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
  }
  .header-top h3 {
    margin: 0;
  }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .summary-text {
    font-size: 12px;
  }
  .bottle-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .bottle-row label {
    font-size: 12px;
  }
  .bottle-select {
    width: auto;
    min-height: 30px;
    padding: 3px 8px;
    font-size: 13px;
  }
  .screen-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 16px 28px 24px;
  }
  .group-header {
    margin: 18px 0 4px;
    color: var(--color-accent-700);
  }
  .rows > .group-header:first-child {
    margin-top: 0;
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
  .fix-error-banner {
    position: fixed;
    left: 50%;
    bottom: 20px;
    transform: translateX(-50%);
    background: var(--color-bg);
    border: 1px solid var(--color-divider);
    padding: 8px 14px;
    font-size: 12.5px;
    color: var(--color-accent-900);
    box-shadow: var(--shadow-md);
    z-index: 45;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 460px;
  }
  .fix-error-remedy {
    font-size: 11.5px;
    color: color-mix(in srgb, var(--color-text) 60%, transparent);
  }
  .fix-error-retry {
    align-self: flex-start;
    font-size: 11.5px;
    padding: 2px 8px;
    margin-top: 2px;
  }
  .confirm-backdrop {
    position: fixed;
    inset: 0;
    z-index: 45;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--color-neutral-900) 45%, transparent);
  }
  .confirm-dialog {
    width: 380px;
    max-width: calc(100vw - 32px);
    background: var(--color-bg);
  }
  .confirm-dialog .dialog-body {
    margin: 0;
  }
</style>
