<script lang="ts">
  // Add/edit one library entry — Identity & paths (editable + debounced
  // `validateGame`), Patches and Streaming (v1 trim: read-only display, the
  // global runtime values live in Settings), and per-flag launch overrides.
  // `gameId === null` is the "Add game" path (starts from
  // `newGameTemplate()`); otherwise this edits a saved entry in place.
  // Structure/classes follow the mockup (Sabrage.dc.html lines 132-222); the
  // load/save/validate state pattern follows Doctor.svelte/Session.svelte.

  import { onMount } from "svelte";
  import {
    isLivePhase,
    suggestBsDir,
    getAppState,
    newGameTemplate,
    pickFolder,
    revertOriginalSteamDll,
    validateGame,
    type GameEntry,
    type GameValidity,
    type GoldbergState,
    type RevertReport,
  } from "../ipc";
  import { configStore } from "../stores/config.svelte";
  import { libraryStore } from "../stores/library.svelte";
  import { sessionStore } from "../stores/session.svelte";
  import { settingsStore } from "../stores/settings.svelte";
  import type { Screen } from "../types";

  interface Props {
    /** `null` = Add game (unsaved, from `newGameTemplate()`). Otherwise the
     * saved entry's id — looked up in `libraryStore.rows` first (already
     * fresh from the Library screen's own `refresh()`), falling back to a
     * `refresh()` of our own if it isn't there yet. */
    gameId: string | null;
    onDone: () => void;
    onNavigate?: (screen: Screen) => void;
  }
  let { gameId, onDone, onNavigate }: Props = $props();

  const GOLDBERG_LABEL: Record<GoldbergState, string> = {
    applied: "applied",
    appliedUnverified: "applied — no .orig-steam backup",
    original: "original steam_api64.dll still present",
    modified: "unrecognized dll — reapplied at next launch",
    noDll: "no steam_api64.dll found yet",
  };

  // ── load ─────────────────────────────────────────────────────────────────

  let loading = $state(true);
  let loadError = $state<string | null>(null);
  /** The working copy — deep-cloned off the store/template so edits here
   * never mutate `libraryStore.rows` before Save. */
  let entry = $state<GameEntry | null>(null);
  /** `entry.bsDir` as loaded (persisted) — `revertOriginalSteamDll` mutates
   * the row's *saved* `bsDir`, not this unsaved draft (see `doRevert`'s
   * `expectedBsDir`, which the backend fails closed against on a mismatch).
   * `null` for the Add-game path (`gameId === null`), where Revert never
   * renders at all. */
  let loadedBsDir = $state<string | null>(null);

  let bottles = $state<string[]>([]);
  let bottlesLoaded = $state(false);

  onMount(async () => {
    try {
      const state = await getAppState();
      bottles = state.bottles;
    } catch {
      bottles = [];
    } finally {
      bottlesLoaded = true;
    }

    try {
      if (gameId) {
        let row = libraryStore.byId(gameId);
        if (!row) {
          await libraryStore.refresh();
          row = libraryStore.byId(gameId);
        }
        if (!row) {
          loadError = "This game is no longer in the library.";
          return;
        }
        entry = structuredClone(row.entry);
        loadedBsDir = row.entry.bsDir;
      } else {
        entry = await newGameTemplate();
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }

    if (!configStore.view && !configStore.loading) void configStore.load();
    if (!settingsStore.loaded) void settingsStore.load();
  });

  // ── validation (debounced 300 ms) ───────────────────────────────────────

  let validity = $state<GameValidity | null>(null);
  let validating = $state(false);
  let validateError = $state<string | null>(null);
  let validateTimer: ReturnType<typeof setTimeout> | null = null;

  async function runValidate() {
    if (!entry) return;
    validating = true;
    validateError = null;
    try {
      validity = await validateGame(entry.bsDir, entry.bottle);
    } catch (e) {
      validateError = e instanceof Error ? e.message : String(e);
    } finally {
      validating = false;
    }
  }

  $effect(() => {
    if (!entry) return;
    // Reactive dependency: re-runs whenever either field changes.
    void entry.bsDir;
    void entry.bottle;
    if (validateTimer) clearTimeout(validateTimer);
    validateTimer = setTimeout(() => void runValidate(), 300);
    // Disarm on teardown: Cancel/Save/"Open Settings" within 300 ms of the last
    // keystroke would otherwise fire a `validate_game` for a screen that no
    // longer exists and write `$state` on a destroyed component.
    return () => {
      if (validateTimer) clearTimeout(validateTimer);
      validateTimer = null;
    };
  });

  async function browseDir() {
    if (!entry) return;
    try {
      // `open()` rejects (it does not resolve null) when the dialog capability
      // is missing or the panel fails, so an unhandled rejection here would
      // leave Browse… looking like a dead button.
      // Start in the field's own dir, else the bottle's derived Beat Saber
      // path (nearest existing ancestor), never wherever macOS last was.
      const suggestion = await suggestBsDir(entry.bottle || null, entry.bsDir || null);
      const picked = await pickFolder("Choose the Beat Saber install directory", suggestion.browseStart);
      if (picked) entry.bsDir = picked;
    } catch (e) {
      validateError = e instanceof Error ? e.message : String(e);
    }
  }

  // ── launch overrides (three-way: use global / on / off) ────────────────

  type OverrideChoice = "global" | "on" | "off";
  function toChoice(v: boolean | null): OverrideChoice {
    return v === null ? "global" : v ? "on" : "off";
  }
  function fromChoice(c: OverrideChoice): boolean | null {
    return c === "global" ? null : c === "on";
  }

  let noAudioChoice = $state<OverrideChoice>("global");
  let noDashboardChoice = $state<OverrideChoice>("global");
  let wiredChoice = $state<OverrideChoice>("global");
  let verboseChoice = $state<OverrideChoice>("global");

  // Seed the four local selects once, the moment `entry` first resolves —
  // not an $effect (which would also fire every time the choices themselves
  // change and immediately overwrite `entry.launchOverrides` right back with
  // its own `fromChoice`, which is harmless but pointless); a one-shot
  // `$effect.pre`-free plain effect keyed on `entry` becoming non-null.
  let seeded = false;
  $effect(() => {
    if (entry && !seeded) {
      seeded = true;
      noAudioChoice = toChoice(entry.launchOverrides.noAudio);
      noDashboardChoice = toChoice(entry.launchOverrides.noDashboard);
      wiredChoice = toChoice(entry.launchOverrides.wired);
      verboseChoice = toChoice(entry.launchOverrides.verbose);
    }
  });

  $effect(() => {
    if (!entry || !seeded) return;
    entry.launchOverrides.noAudio = fromChoice(noAudioChoice);
    entry.launchOverrides.noDashboard = fromChoice(noDashboardChoice);
    entry.launchOverrides.wired = fromChoice(wiredChoice);
    entry.launchOverrides.verbose = fromChoice(verboseChoice);
  });

  // ── revert original steam_api64.dll ─────────────────────────────────────

  // The backend deliberately leaves `phase` at `"exited"` after a session
  // ends until the next launch — `!== "idle"` disabled Revert forever after
  // one clean run, even though that run is exactly what creates the
  // `.orig-steam` backup this button restores. `isLivePhase` is the same
  // shared predicate `blocksMutation`/Session.svelte use, and excludes
  // `"exited"`.
  /** Has the draft path diverged from the persisted row Revert would
   * actually target? Purely advisory — a proactive hint, not the safety
   * boundary: the backend refuses the mismatch either way (see `doRevert`).
   */
  const bsDirDirty = $derived(!!entry && loadedBsDir !== null && entry.bsDir !== loadedBsDir);

  const canRevert = $derived(!!validity?.origSteamPresent && !isLivePhase(sessionStore.status.phase));

  let revertConfirm = $state(false);
  let reverting = $state(false);
  let revertReport = $state<RevertReport | null>(null);
  let revertError = $state<string | null>(null);

  async function doRevert() {
    if (!gameId || !entry) return;
    reverting = true;
    revertError = null;
    try {
      // `entry.bsDir` is the draft path this form validated and displayed —
      // pass it as `expectedBsDir` so the backend fails closed if it differs
      // from the *persisted* row's `bsDir` (what it would actually mutate),
      // rather than silently reverting a different installation than the one
      // on screen. See `revertOriginalSteamDll`'s doc comment.
      revertReport = await revertOriginalSteamDll(gameId, entry.bsDir);
      revertConfirm = false;
      void runValidate();
    } catch (e) {
      revertError = e instanceof Error ? e.message : String(e);
    } finally {
      reverting = false;
    }
  }

  // ── save / cancel ────────────────────────────────────────────────────────

  let saving = $state(false);
  let saveError = $state<string | null>(null);

  const canSave = $derived(
    !saving && !!entry && entry.name.trim().length > 0 && entry.bsDir.trim().length > 0 && !!entry.bottle,
  );

  async function doSave() {
    if (!entry || !canSave) return;
    saving = true;
    saveError = null;
    try {
      const row = await libraryStore.save(entry);
      loadedBsDir = row.entry.bsDir;
      onDone();
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="screen-header">
  <h3 class="header-title">
    <button class="btn btn-ghost btn-icon back-btn" onclick={onDone} aria-label="Back to Library">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"
        ><line x1="19" y1="12" x2="5" y2="12"></line><polyline points="12 19 5 12 12 5"></polyline></svg
      >
    </button>
    {gameId ? `Edit — ${entry?.name ?? ""}` : "Add game"}
  </h3>
  <div class="header-actions">
    <button class="btn btn-secondary" onclick={onDone}>Cancel</button>
    <button class="btn btn-primary" onclick={doSave} disabled={!canSave}>
      {saving ? "Saving…" : "Save configuration"}
    </button>
  </div>
</div>

<div class="screen-body">
  {#if loading}
    <p class="text-muted">Loading…</p>
  {:else if loadError}
    <div class="blueprint error-card">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <h6>Could not open this game</h6>
      <p class="text-muted">{loadError}</p>
      <button class="btn btn-secondary" onclick={onDone}>Back to Library</button>
    </div>
  {:else if entry}
    <div class="edit-grid">
      <div class="col">
        <div class="blueprint card-panel">
          <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
          <h6 class="card-title-accent">Identity &amp; paths</h6>

          <div class="field">
            <label for="edit-name">Display name</label>
            <input id="edit-name" class="input" type="text" bind:value={entry.name} />
          </div>

          <div class="field">
            <label for="edit-dir">Install dir (the folder containing the .exe)</label>
            <div class="row-inline">
              <input id="edit-dir" class="input mono" type="text" bind:value={entry.bsDir} />
              <button class="btn btn-secondary browse-btn" onclick={browseDir}>Browse…</button>
            </div>
            <div class="text-muted hint">
              A folder outside the bottle is reached through its z: drive — Doctor checks it.
            </div>
          </div>

          <div class="field">
            <label for="edit-bottle">CrossOver bottle</label>
            {#if bottlesLoaded && bottles.length === 0}
              <span class="text-muted">none found — create one in the CrossOver UI</span>
            {:else}
              <select id="edit-bottle" class="input" bind:value={entry.bottle}>
                {#each bottles as b (b)}
                  <option value={b}>{b}</option>
                {/each}
              </select>
            {/if}
          </div>

          <div class="field appid-field">
            <label for="edit-appid">Steam App ID</label>
            <input id="edit-appid" class="input appid-input" type="number" value={entry.appid} disabled />
            <div class="text-muted hint">
              Fixed in v1: every launch writes the pipeline contract's App ID to steam_appid.txt, the same
              value ./demo.sh run writes.
            </div>
          </div>

          {#if validating}
            <div class="text-muted validating-note">Validating…</div>
          {:else if validateError}
            <div class="validate-error">Could not validate: {validateError}</div>
          {:else if validity}
            <div class="validity-block">
              <div class="text-muted version-line">
                Detected version: {validity.detectedVersion ?? "unknown"}
                {#if validity.detectedVersion && !validity.versionOk}
                  <span class="validity-warn"> — expected 1.29.4</span>
                {/if}
              </div>
              {#if validity.outsideDriveC}
                <div class="text-muted outside-note">
                  Outside the bottle — reached through its z: drive{validity.zDriveOk === false
                    ? " (missing — Doctor will flag this)"
                    : ""}.
                </div>
              {/if}
              {#each validity.problems as problem, i (i)}
                <div class="validity-problem">{problem}</div>
              {/each}
            </div>
          {/if}
        </div>

        <div class="blueprint card-panel">
          <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
          <h6 class="card-title-accent">Patches</h6>
          <p class="patch-line">
            Goldberg steam_api64.dll — applied at every launch{#if validity}
              <span class="text-muted">({GOLDBERG_LABEL[validity.goldberg]})</span>
            {/if}
          </p>
          <ul class="patch-sub-list text-muted">
            <li>steam_settings/offline.txt</li>
            <li>steam_settings/disable_networking.txt</li>
            <li>steam_settings/disable_overlay.txt</li>
          </ul>
          <p class="text-muted appid-note">
            steam_appid.txt is written at every launch with the pipeline contract's App ID — Sabrage
            never writes a per-game value.
          </p>

          {#if gameId}
            <div class="hr"></div>
            {#if revertConfirm}
              <div class="revert-confirm">
                <span class="text-muted">Restore the original steam_api64.dll?</span>
                <button class="btn btn-secondary" onclick={() => (revertConfirm = false)} disabled={reverting}
                  >Cancel</button
                >
                <button class="btn btn-primary" onclick={doRevert} disabled={reverting}>
                  {reverting ? "Reverting…" : "Yes, revert"}
                </button>
              </div>
            {:else}
              <button class="btn btn-secondary" disabled={!canRevert} onclick={() => (revertConfirm = true)}>
                Revert original steam_api64.dll
              </button>
              {#if validity && !canRevert && validity.origSteamPresent && isLivePhase(sessionStore.status.phase)}
                <div class="text-muted revert-note">A session is live — stop it first.</div>
              {:else if canRevert && bsDirDirty}
                <div class="text-muted revert-note">
                  Save your path change first — Revert acts on the saved install dir ({loadedBsDir}), not this
                  unsaved edit.
                </div>
              {/if}
            {/if}
            {#if revertReport}
              <div class="text-muted revert-report">{revertReport.message}</div>
            {/if}
            {#if revertError}
              <div class="validate-error">{revertError}</div>
            {/if}
          {/if}
        </div>
      </div>

      <div class="col">
        <div class="blueprint card-panel">
          <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
          <h6 class="card-title-accent">Streaming</h6>
          {#if configStore.loading && !configStore.view}
            <p class="text-muted">Loading…</p>
          {:else if configStore.error}
            <p class="validate-error">{configStore.error}</p>
          {:else if configStore.view}
            {@const v = configStore.view}
            <div class="stream-values">
              <div class="stream-row">
                <span class="stream-label">Protocol</span>
                <span>{v.values.protocol ?? `runtime default: ${v.defaults.protocol}`}</span>
              </div>
              <div class="stream-row">
                <span class="stream-label">Bitrate</span>
                <span>
                  {v.values.bitrateMbps != null
                    ? `${v.values.bitrateMbps} Mbps`
                    : `runtime default: ${v.defaults.bitrateMbps} Mbps`}
                </span>
              </div>
              <div class="stream-row">
                <span class="stream-label">Encoder process</span>
                <span>{v.values.encoderProcess ?? `runtime default: ${v.defaults.encoderProcess}`}</span>
              </div>
              <div class="stream-row">
                <span class="stream-label">Video codec</span>
                <span>{v.values.videoCodec ?? `runtime default: ${v.defaults.videoCodec}`}</span>
              </div>
              <div class="stream-row">
                <span class="stream-label">Resolution scale</span>
                <span>{v.values.resolutionScale ?? `runtime default: ${v.defaults.resolutionScale}`}</span>
              </div>
              <div class="stream-row">
                <span class="stream-label">Refresh rate</span>
                <span>
                  {v.values.refreshRateHz != null
                    ? `${v.values.refreshRateHz} Hz`
                    : `runtime default: ${v.defaults.refreshRateHz} Hz`}
                </span>
              </div>
            </div>
          {/if}
          <button class="btn btn-ghost open-settings-btn" onclick={() => onNavigate?.("settings")}>
            Open Settings
          </button>
          <div class="hr"></div>
          <div class="text-muted streaming-note">
            These are global — the runtime toml at ~/Library/Application Support/OXRSys/ stays the single source;
            this game has no per-game streaming values in v1.
          </div>
        </div>

        <div class="blueprint card-panel">
          <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
          <h6 class="card-title-accent">Launch overrides</h6>
          <p class="text-muted overrides-note">Per-game overrides of the four demo.sh flags in Settings.</p>

          <div class="field">
            <label for="ov-audio">No audio routing</label>
            <select id="ov-audio" class="input" bind:value={noAudioChoice}>
              <option value="global">Use global</option>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </div>
          <div class="field">
            <label for="ov-dashboard">No dashboard</label>
            <select id="ov-dashboard" class="input" bind:value={noDashboardChoice}>
              <option value="global">Use global</option>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </div>
          <div class="field">
            <label for="ov-wired">Wired (USB)</label>
            <select id="ov-wired" class="input" bind:value={wiredChoice}>
              <option value="global">Use global</option>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </div>
          <div class="field">
            <label for="ov-verbose">Verbose wine log</label>
            <select id="ov-verbose" class="input" bind:value={verboseChoice}>
              <option value="global">Use global</option>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </div>
        </div>
      </div>
    </div>

    {#if saveError}
      <div class="save-error">Save failed: {saveError}</div>
    {/if}
  {/if}
</div>

<style>
  .screen-header {
    padding: 22px 28px 14px;
    border-bottom: 1px solid var(--color-divider);
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
  }
  .header-title {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .back-btn {
    width: 30px;
    height: 30px;
  }
  .header-actions {
    display: flex;
    gap: 8px;
    flex: none;
  }
  .screen-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 22px 28px;
  }
  .error-card {
    padding: 18px 20px;
    max-width: 460px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }
  .error-card h6 {
    color: var(--color-accent-900);
    margin-bottom: 0;
  }
  .error-card p {
    font-size: 13px;
    margin: 0;
  }
  .edit-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 22px;
    align-items: start;
  }
  @media (max-width: 860px) {
    .edit-grid {
      grid-template-columns: 1fr;
    }
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 22px;
    min-width: 0;
  }
  .card-panel {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .card-title-accent {
    color: var(--color-accent-700);
    margin-bottom: 0;
  }
  .row-inline {
    display: flex;
    gap: 8px;
  }
  .row-inline .input {
    flex: 1;
    min-width: 0;
  }
  .browse-btn {
    flex: none;
  }
  .mono {
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11.5px;
  }
  .hint {
    font-size: 11px;
    margin-top: 3px;
  }
  .appid-field {
    max-width: 200px;
  }
  .appid-input {
    max-width: 160px;
  }
  .validity-block {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 12.5px;
  }
  .validate-error,
  .validity-problem {
    font-size: 12px;
    color: var(--color-accent-900);
  }
  .validity-warn {
    color: var(--color-accent-900);
  }
  .outside-note {
    font-size: 11.5px;
  }
  .patch-line {
    font-size: 13.5px;
    margin: 0;
  }
  .patch-sub-list {
    margin: 2px 0 0;
    padding-left: 18px;
    font-size: 11.5px;
    font-family: ui-monospace, Menlo, monospace;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .appid-note {
    font-size: 11px;
    margin: 0;
  }
  .revert-confirm {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .revert-report,
  .revert-note {
    font-size: 11.5px;
    margin-top: 6px;
  }
  .stream-values {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  .stream-row {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }
  .stream-label {
    color: color-mix(in srgb, var(--color-text) 60%, transparent);
  }
  .open-settings-btn {
    align-self: flex-start;
    font-size: 12.5px;
    padding: 2px 4px;
  }
  .streaming-note {
    font-size: 11.5px;
  }
  .overrides-note {
    font-size: 11.5px;
    margin: -2px 0 4px;
  }
  .save-error {
    margin-top: 16px;
    font-size: 12.5px;
    color: var(--color-accent-900);
  }
</style>
