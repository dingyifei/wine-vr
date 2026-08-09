<script lang="ts">
  import { onMount } from "svelte";
  import CheckRow from "../components/CheckRow.svelte";
  import { doctorStore } from "../stores/doctor.svelte";

  let selectedBottle = $state("");

  function pickDefaultBottle(bottles: string[]): string {
    return bottles.includes("Steam") ? "Steam" : (bottles[0] ?? "");
  }

  onMount(async () => {
    await doctorStore.loadBottles();
    selectedBottle = pickDefaultBottle(doctorStore.bottles);
    void runChecks();
  });

  async function runChecks() {
    await doctorStore.run({ bottle: selectedBottle || null });
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
        <CheckRow {row} isRunning={row.slug === doctorStore.runningSlug} />
      {/each}
    </div>
  {/if}
</div>

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
</style>
