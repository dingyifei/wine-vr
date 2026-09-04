<script lang="ts">
  /** Owns the Logs screen: selected tab, live tail buffer (`lines` and its
   * lowercased twin `lowerLines`), filter, and past-runs listing. Holds the
   * tail id and talks to the backend over log-tail IPC directly, so every
   * navigation and unmount must stop the current tail before starting
   * another. */
  import { onDestroy, onMount } from "svelte";
  import { errMsg } from "../lib/text";
  import {
    getLogSourcePath,
    listPastRuns,
    startLogTail,
    stopLogTail,
    type LogBatch,
    type LogSource,
    type PastRun,
  } from "../ipc";

  type Tab = "wineConsole" | "oxrsysRuntime" | "alvrSession" | "pastRuns";

  const TABS: { id: Tab; label: string }[] = [
    { id: "wineConsole", label: "Wine console" },
    { id: "oxrsysRuntime", label: "oxrsys runtime" },
    { id: "alvrSession", label: "ALVR session" },
    { id: "pastRuns", label: "Past runs" },
  ];

  const MAX_LINES = 5000;
  /** Hysteresis over `MAX_LINES`: a trim re-slices both `lines` and
   * `lowerLines`, so it runs about once per 1000 lines instead of on every
   * batch once the buffer sits at cap. */
  const TRIM_BLOCK = 1000;
  /** Debounce before `filterQuery` — and so `filteredLines` — follows a
   * keystroke: a non-empty query rescans the whole buffer, a cost every
   * incoming batch already pays again while a filter is set. */
  const FILTER_DEBOUNCE_MS = 150;

  let tab = $state<Tab>("wineConsole");
  /** The last tab a navigation was *requested* for. `switchTab`'s
   * "already there, no-op" guard reads this and not `tab`, which updates only
   * once the in-flight `stopTail()` resolves; a rapid A -> B -> A would land on B. */
  let pendingTab = $state<Tab>("wineConsole");
  let follow = $state(true);
  /** Bound to the filter `<input>` directly — updates every keystroke. */
  let filter = $state("");
  /** The debounced value `filteredLines` actually filters against. */
  let filterQuery = $state("");
  let filterDebounceHandle: ReturnType<typeof setTimeout> | null = null;
  let lines = $state<string[]>([]);
  /** Parallel to `lines`, each entry already lowercased at push time — so
   * filtering rescans without re-lowercasing the whole buffer on every pass. */
  let lowerLines = $state<string[]>([]);
  let rotatedNotice = $state(false);
  let truncatedNotice = $state(false);
  let resolvedPath = $state<string | null>(null);
  let pathError = $state<string | null>(null);
  let tailId: number | null = null;
  /** Guards `switchTab`/`openPastRun`/`backToPastRuns`: a `stopTail()` await
   * that settles after a later navigation superseded it must not assign
   * `tab`/`openedPastRun` or start a tail for the navigation already left. */
  let navGeneration = 0;
  /** Guards a leaked tail: `startTail` captures this at its start and skips
   * every shared-state write it would make, `tailId` included, once a later
   * `startTail`/`stopTail` bumped it. Only the id in `tailId` is ever stopped. */
  let tailGeneration = 0;
  let pastRuns = $state<PastRun[]>([]);
  let pastRunsLoaded = $state(false);
  let openedPastRun = $state<PastRun | null>(null);

  let preEl: HTMLPreElement | null = $state(null);

  function sourceForTab(t: Exclude<Tab, "pastRuns">): LogSource {
    switch (t) {
      case "wineConsole":
        return { kind: "wineConsole" };
      case "oxrsysRuntime":
        return { kind: "oxrsysRuntime" };
      case "alvrSession":
        return { kind: "alvrSession" };
    }
  }

  function onBatch(b: LogBatch) {
    if (b.rotated) {
      lines = [];
      lowerLines = [];
      rotatedNotice = true;
    }
    if (b.truncated) truncatedNotice = true;
    resolvedPath = b.path;
    if (b.lines.length > 0) {
      lines.push(...b.lines);
      lowerLines.push(...b.lines.map((l) => l.toLowerCase()));
      if (lines.length > MAX_LINES + TRIM_BLOCK) {
        lines = lines.slice(lines.length - MAX_LINES);
        lowerLines = lowerLines.slice(lowerLines.length - MAX_LINES);
      }
    }
    if (follow) {
      queueMicrotask(() => {
        if (preEl) preEl.scrollTop = preEl.scrollHeight;
      });
    }
  }

  async function startTail(source: LogSource) {
    const myGeneration = ++tailGeneration;
    lines = [];
    lowerLines = [];
    rotatedNotice = false;
    truncatedNotice = false;
    resolvedPath = null;
    pathError = null;
    // Generation-check this call's batches too: one landing between
    // `startLogTail` resolving and the stale-case `stopLogTail` below would
    // otherwise mix into whatever tab is current by then.
    const handleBatch = (b: LogBatch) => {
      if (myGeneration === tailGeneration) onBatch(b);
    };
    try {
      const path = await getLogSourcePath(source);
      if (myGeneration !== tailGeneration) return;
      resolvedPath = path;
    } catch (e) {
      if (myGeneration !== tailGeneration) return;
      pathError = errMsg(e);
    }
    try {
      const id = await startLogTail(source, handleBatch);
      if (myGeneration !== tailGeneration) {
        // A newer switchTab/openPastRun/stopTail call superseded this one
        // while startLogTail was in flight — stop the tail we just started
        // instead of leaking it (see `tailGeneration`'s doc comment).
        void stopLogTail(id).catch(() => {});
        return;
      }
      tailId = id;
    } catch (e) {
      if (myGeneration !== tailGeneration) return;
      pathError = errMsg(e);
    }
  }

  async function stopTail() {
    // Bump first, unconditionally: this invalidates a `startTail` still in
    // flight even before it reaches `startLogTail`, and even when no
    // replacement `startTail` follows (the tail-less "Past runs" tab).
    tailGeneration++;
    if (tailId != null) {
      const id = tailId;
      tailId = null;
      try {
        await stopLogTail(id);
      } catch {
        // best-effort — the pane is closing anyway
      }
    }
  }

  async function loadPastRuns() {
    pastRunsLoaded = false;
    try {
      pastRuns = await listPastRuns();
    } finally {
      pastRunsLoaded = true;
    }
  }

  async function switchTab(next: Tab) {
    // Past runs is the one tab worth re-clicking: it refreshes the listing.
    // Compares `pendingTab`, not `tab`, which lags (see `pendingTab`'s doc).
    if (next === pendingTab && next !== "pastRuns") return;
    pendingTab = next;
    const myNav = ++navGeneration;
    await stopTail();
    // A later navigation superseded this one while `stopTail()` was in
    // flight; let its own continuation own `tab`/`openedPastRun` and the tail.
    if (myNav !== navGeneration) return;
    tab = next;
    openedPastRun = null;
    if (next === "pastRuns") {
      void loadPastRuns();
      return;
    }
    void startTail(sourceForTab(next));
  }

  async function openPastRun(run: PastRun) {
    const myNav = ++navGeneration;
    await stopTail();
    if (myNav !== navGeneration) return;
    openedPastRun = run;
    void startTail({ kind: "file", path: run.path });
  }

  async function backToPastRuns() {
    const myNav = ++navGeneration;
    await stopTail();
    if (myNav !== navGeneration) return;
    openedPastRun = null;
    lines = [];
    lowerLines = [];
  }

  onMount(() => {
    void startTail(sourceForTab("wineConsole"));
  });

  onDestroy(() => {
    void stopTail();
    if (filterDebounceHandle) clearTimeout(filterDebounceHandle);
  });

  $effect(() => {
    const f = filter;
    if (filterDebounceHandle) clearTimeout(filterDebounceHandle);
    filterDebounceHandle = setTimeout(() => {
      filterDebounceHandle = null;
      filterQuery = f;
    }, FILTER_DEBOUNCE_MS);
  });

  const filteredLines = $derived.by(() => {
    const q = filterQuery.trim().toLowerCase();
    if (!q) return lines;
    const out: string[] = [];
    for (let i = 0; i < lines.length; i++) {
      if (lowerLines[i].includes(q)) out.push(lines[i]);
    }
    return out;
  });

  const showTailPane = $derived(tab !== "pastRuns" || openedPastRun !== null);

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  function formatTime(unixMs: number): string {
    return new Date(unixMs).toLocaleString();
  }
</script>

<div class="screen-header">
  <div class="header-top">
    <h3>Logs</h3>
  </div>
  <div class="tabs">
    {#each TABS as t (t.id)}
      <button class="tab-btn" class:active={tab === t.id} onclick={() => void switchTab(t.id)}>{t.label}</button>
    {/each}
  </div>
  {#if showTailPane}
    <div class="path-row">
      <span class="text-muted path-label">
        {#if openedPastRun}
          {openedPastRun.fileName}
        {:else if pathError}
          {pathError}
        {:else}
          {resolvedPath ?? "—"}
        {/if}
      </span>
      {#if openedPastRun}
        <button class="btn btn-ghost back-btn" onclick={() => void backToPastRuns()}>&larr; Past runs</button>
      {/if}
      <label class="follow-toggle">
        <input type="checkbox" bind:checked={follow} />
        Follow
      </label>
      <input class="input filter-input" type="text" placeholder="Filter…" bind:value={filter} />
    </div>
    {#if rotatedNotice}
      <div class="notice">log rotated — showing the new file from the start</div>
    {/if}
    {#if truncatedNotice}
      <div class="notice">the file is growing faster than Sabrage reads it — more lines are on the way</div>
    {/if}
  {/if}
</div>

<div class="screen-body">
  {#if tab === "pastRuns" && !openedPastRun}
    <div class="past-runs">
      {#if !pastRunsLoaded}
        <p class="text-muted">Loading…</p>
      {:else if pastRuns.length === 0}
        <p class="text-muted">No past runs in logs/ yet.</p>
      {:else}
        <div class="table-wrap">
          <table class="table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Size</th>
                <th>Modified</th>
              </tr>
            </thead>
            <tbody>
              {#each pastRuns as run (run.path)}
                <tr class="past-run-row" onclick={() => void openPastRun(run)}>
                  <td>{run.fileName}</td>
                  <td>{formatSize(run.size)}</td>
                  <td>{formatTime(run.modifiedUnixMs)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
  {:else if showTailPane}
    {#if filteredLines.length === 0}
      <p class="text-muted empty-note">
        {lines.length === 0 ? "No lines yet." : "No lines match the filter."}
      </p>
    {/if}
    <pre class="log-pane" bind:this={preEl}>{filteredLines.join("\n")}</pre>
  {/if}
</div>

<style>
  .screen-header {
    padding: 18px 28px 12px;
    border-bottom: 1px solid var(--color-divider);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .header-top h3 {
    margin: 0;
  }
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tab-btn {
    font-family: var(--font-heading);
    font-weight: 600;
    font-size: 12.5px;
    padding: 5px 12px;
    border: 1px solid var(--color-divider);
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }
  .tab-btn.active {
    background: var(--color-accent);
    color: var(--color-bg);
    border-color: var(--color-accent);
  }
  .tab-btn:not(.active):hover {
    background: color-mix(in srgb, var(--color-text) 6%, transparent);
  }
  .path-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .path-label {
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11.5px;
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .back-btn {
    flex: none;
    font-size: 11.5px;
    padding: 2px 6px;
  }
  .follow-toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    flex: none;
    cursor: pointer;
  }
  .follow-toggle input {
    accent-color: var(--color-accent);
  }
  .filter-input {
    width: 180px;
    min-height: 26px;
    padding: 3px 8px;
    font-size: 12px;
    flex: none;
  }
  .notice {
    font-size: 11.5px;
    color: color-mix(in srgb, var(--color-text) 60%, transparent);
  }
  .screen-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 28px 20px;
    display: flex;
    flex-direction: column;
  }
  .empty-note {
    margin: 14px 0 0;
    font-size: 13px;
  }
  .log-pane {
    flex: 1;
    min-height: 0;
    margin: 12px 0 0;
    padding: 12px 14px;
    background: var(--color-surface);
    border: 1px solid var(--color-divider);
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    overflow: auto;
  }
  .past-runs {
    padding-top: 14px;
  }
  .table-wrap {
    overflow-x: auto;
  }
  .past-run-row {
    cursor: pointer;
  }
</style>
