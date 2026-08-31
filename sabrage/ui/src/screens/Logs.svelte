<script lang="ts">
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
  /** How far past `MAX_LINES` the buffer is allowed to grow before it gets
   * trimmed back down, so the trim (a full array re-slice, done to both
   * `lines` and `lowerLines`) fires roughly once every 1000 lines instead of
   * reallocating on every batch once the buffer is at cap. */
  const TRIM_BLOCK = 1000;
  /** How long to hold a keystroke before it actually re-runs the filter —
   * `filteredLines` rescans the whole buffer, and without this every
   * keystroke (and, while a filter is set, every incoming batch too) paid
   * that cost immediately. */
  const FILTER_DEBOUNCE_MS = 150;

  let tab = $state<Tab>("wineConsole");
  /** Tracks the last tab a switch/back navigation was actually requested for
   * — read instead of `tab` itself by the "already there, no-op" checks below,
   * since `tab` only updates once the in-flight `stopTail()` resolves. Using
   * the stale `tab` there let a rapid A -> B -> A get treated as a no-op on
   * the second click and land on B instead of the user's actual last choice. */
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
  /** Guards against a switchTab/openPastRun/backToPastRuns call whose
   * `stopTail()` await settles after a later navigation call already
   * superseded it — without this, an out-of-order resolution could still
   * assign `tab`/`openedPastRun` and start a tail for the navigation the user
   * had already moved past. */
  let navGeneration = 0;
  /**
   * Guards against a leaked tail from two interleaved `startTail`/`stopTail`
   * calls (rapid tab switching): `startTail` captures the generation current
   * at its own start, and every mutation it makes to shared state — including
   * the eventual `tailId = id` assignment — is skipped once a later
   * `startTail`/`stopTail` call has bumped the counter past it. Without this,
   * an out-of-order `startLogTail` resolution (tab A's IPC call outlives tab
   * B's) clobbers `tailId` with A's id while B's tail is the one actually
   * feeding `lines` — B's tail then never gets stopped, since only the id in
   * `tailId` is ever passed to `stopLogTail`.
   */
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
    // Route this call's batches through a generation check too — otherwise a
    // batch that lands between `startLogTail` resolving and the immediate
    // `stopLogTail` below (stale case) would still reach `onBatch` and mix
    // into whatever tab is current by then.
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
    // Bump first, unconditionally: this invalidates any `startTail` still in
    // flight even when it hasn't reached `startLogTail` yet (e.g. still
    // awaiting `getLogSourcePath`), and even when no replacement `startTail`
    // follows (switching to the tail-less "Past runs" tab).
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
    // Everything else is already live-tailing, so re-selecting it is a no-op.
    // Compares against `pendingTab` (the last *requested* tab), not `tab`
    // itself — `tab` only updates once `stopTail()` below resolves, so a
    // rapid A -> B -> A would otherwise see the second click's `next` (A)
    // still equal to the still-stale `tab` (A) and treat it as a no-op,
    // leaving the switch to land on B instead of the user's actual last pick.
    if (next === pendingTab && next !== "pastRuns") return;
    pendingTab = next;
    const myNav = ++navGeneration;
    await stopTail();
    // A newer switchTab/openPastRun/backToPastRuns call superseded this one
    // while `stopTail()` was in flight — let that call's own continuation own
    // `tab`/`openedPastRun` and the resulting tail instead of overwriting it.
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
