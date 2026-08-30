<script lang="ts">
  // Phase 4 — mockup: docs/design/mockup/Sabrage.dc.html (SETTINGS, lines 391–460;
  // mock state 850–915). Design source: docs/design/design-app.md §4 screen table +
  // "Settings write policy" paragraph, design-core.md §4.
  //
  // Two independent persistence stores back this screen:
  //  - `configStore` (oxrsys-runtime.toml, via config/runtime_toml.rs) — the
  //    Streaming card only. Edited as a local `draft` diffed against the loaded
  //    view so Save writes only changed keys (a `null`/untouched field is never
  //    sent — see `buildPatch`); explicit Save/Revert, gated behind a one-time
  //    inline acknowledgement panel (never `window.confirm` — it freezes the
  //    webview).
  //  - `settingsStore` (settings.json) — Audio & launch, Paths, and the adb-probe
  //    toggle. Every field here autosaves on change/blur (mirrors Session.svelte's
  //    local-`$state`-then-store convention), with a small transient "Saved" flash
  //    instead of a form-wide save button.
  import { onMount } from "svelte";
  import {
    getAppState,
    getRepoInfo,
    pickFolder,
    type EncoderProcess,
    type LaunchDefaults,
    type LaunchOpts,
    type Protocol,
    type RepoInfo,
    type RuntimeConfigPatch,
    type RuntimeConfigValues,
    type Settings,
    type VideoCodec,
    type WriteReport,
  } from "../ipc";
  import { demoRunCommand } from "../lib/demo";
  import { configStore } from "../stores/config.svelte";
  import { settingsStore } from "../stores/settings.svelte";
  import { stageStore } from "../stores/stage.svelte";

  // ── bottles (Paths card's default-bottle select) ───────────────────────────

  let bottles = $state<string[]>([]);
  let bottlesLoaded = $state(false);

  // ── settings.json — local mirrors, seeded once settings load, then autosaved ─

  let defaultBottleSel = $state("");
  let bsDirInput = $state("");
  let routeAudioChk = $state(true);
  let openDashboardChk = $state(true);
  let wiredChk = $state(false);
  let verboseChk = $state(false);
  let allowAdbChk = $state(true);

  let savedFlash = $state(false);
  let settingsSaveError = $state<string | null>(null);
  let flashTimer: ReturnType<typeof setTimeout> | null = null;

  function flashSaved() {
    savedFlash = true;
    if (flashTimer) clearTimeout(flashTimer);
    flashTimer = setTimeout(() => (savedFlash = false), 1400);
  }

  function seedFromSettings() {
    const s = settingsStore.settings;
    if (!s) return;
    defaultBottleSel = s.defaultBottle ?? "";
    bsDirInput = s.defaultBsDir ?? "";
    routeAudioChk = !s.launch.noAudio;
    openDashboardChk = !s.launch.noDashboard;
    wiredChk = s.launch.wired;
    verboseChk = s.launch.verbose;
    allowAdbChk = s.allowAdbProbes;
  }

  async function persistSettings(patch: Partial<Settings>) {
    settingsSaveError = null;
    try {
      await settingsStore.update(patch);
      flashSaved();
    } catch (e) {
      settingsSaveError = e instanceof Error ? e.message : String(e);
      seedFromSettings(); // roll the controls back to whatever the store actually kept
    }
  }

  async function persistLaunch(patch: Partial<LaunchDefaults>) {
    const current = settingsStore.settings;
    if (!current) return;
    await persistSettings({ launch: { ...current.launch, ...patch } });
  }

  async function onDefaultBottleChange() {
    await persistSettings({ defaultBottle: defaultBottleSel || null });
  }
  async function onBsDirCommit() {
    const trimmed = bsDirInput.trim();
    await persistSettings({ defaultBsDir: trimmed ? trimmed : null });
  }
  async function browseDefaultBsDir() {
    // `open()` REJECTS (it does not resolve null) when the dialog capability is
    // missing or the panel fails; unhandled, that makes Browse… a dead button
    // with nothing on screen to explain it.
    let dir: string | null;
    try {
      dir = await pickFolder("Choose the default Beat Saber install directory", bsDirInput || null);
    } catch (e) {
      settingsSaveError = `Could not open the folder picker: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    if (!dir) return;
    bsDirInput = dir;
    await onBsDirCommit();
  }
  async function onRouteAudioChange() {
    await persistLaunch({ noAudio: !routeAudioChk });
  }
  async function onOpenDashboardChange() {
    await persistLaunch({ noDashboard: !openDashboardChk });
  }
  async function onWiredChange() {
    await persistLaunch({ wired: wiredChk });
  }
  async function onVerboseChange() {
    await persistLaunch({ verbose: verboseChk });
  }
  async function onAllowAdbChange() {
    await persistSettings({ allowAdbProbes: allowAdbChk });
  }

  // ── oxrsys-runtime.toml — draft diffed against the loaded view ─────────────

  const EMPTY_VALUES: RuntimeConfigValues = {
    protocol: null,
    bitrateMbps: null,
    encoderProcess: null,
    videoCodec: null,
    resolutionScale: null,
    refreshRateHz: null,
  };

  /** `null` = "not set by the user this session" (renders "runtime default: …");
   * seeded from `configStore.view.values` on load/Revert/successful Save, so an
   * untouched key stays `null` and never enters the write patch. */
  let draft = $state<RuntimeConfigValues>({ ...EMPTY_VALUES });

  function resetDraft() {
    draft = configStore.view ? { ...configStore.view.values } : { ...EMPTY_VALUES };
  }

  const RUNTIME_KEYS = [
    "protocol",
    "bitrateMbps",
    "encoderProcess",
    "videoCodec",
    "resolutionScale",
    "refreshRateHz",
  ] as const;
  type RuntimeKey = (typeof RUNTIME_KEYS)[number];

  /** Diff `draft` against the loaded view: a field only enters the patch when it
   * differs from what's on disk — an untouched (still-`null`) field never does,
   * and a field explicitly reset back to its loaded value drops back out. */
  function buildPatch(loaded: RuntimeConfigValues | undefined, current: RuntimeConfigValues): RuntimeConfigPatch {
    const base = loaded ?? EMPTY_VALUES;
    return {
      protocol: current.protocol !== base.protocol ? current.protocol : null,
      bitrateMbps: current.bitrateMbps !== base.bitrateMbps ? current.bitrateMbps : null,
      encoderProcess: current.encoderProcess !== base.encoderProcess ? current.encoderProcess : null,
      videoCodec: current.videoCodec !== base.videoCodec ? current.videoCodec : null,
      resolutionScale: current.resolutionScale !== base.resolutionScale ? current.resolutionScale : null,
      refreshRateHz: current.refreshRateHz !== base.refreshRateHz ? current.refreshRateHz : null,
    };
  }

  const patch = $derived.by((): RuntimeConfigPatch => buildPatch(configStore.view?.values, draft));
  const isDirty = $derived(RUNTIME_KEYS.some((k) => patch[k] !== null));

  const effectiveProtocol = $derived.by(
    (): Protocol | null => draft.protocol ?? configStore.view?.defaults.protocol ?? null,
  );
  const effectiveVideoCodec = $derived.by(
    (): VideoCodec | null => draft.videoCodec ?? configStore.view?.defaults.videoCodec ?? null,
  );
  const effectiveEncoderProcess = $derived.by(
    (): EncoderProcess | null => draft.encoderProcess ?? configStore.view?.defaults.encoderProcess ?? null,
  );
  const effectiveBitrate = $derived(draft.bitrateMbps ?? configStore.view?.defaults.bitrateMbps ?? 50);
  const effectiveResolutionScale = $derived(
    draft.resolutionScale ?? configStore.view?.defaults.resolutionScale ?? 0.75,
  );
  const effectiveRefreshHz = $derived(draft.refreshRateHz ?? configStore.view?.defaults.refreshRateHz ?? 72);
  const resPx = $derived(`${Math.round(3008 * effectiveResolutionScale)}×${Math.round(1664 * effectiveResolutionScale)}`);

  const isLegacyProtocol = $derived(effectiveProtocol === "oxrsys");

  const CODEC_OPTS: { value: VideoCodec; label: string }[] = [
    { value: "auto", label: "auto" },
    { value: "h265", label: "h265" },
    { value: "h264", label: "h264" },
  ];
  const ENCODER_OPTS: { value: EncoderProcess; label: string }[] = [
    { value: "auto", label: "auto" },
    { value: "native", label: "native" },
    { value: "inproc", label: "inproc" },
  ];
  const PROTOCOL_OPTS: { value: Protocol; label: string }[] = [
    { value: "alvr", label: "alvr" },
    // The label carries the consequence, because this radio is the decision
    // point: Sabrage's run preflight blocks the legacy protocol outright
    // (contract native_gate = block; stages/run/preflight.rs), it does not warn.
    { value: "oxrsys", label: "oxrsys (legacy — demo.sh only)" },
  ];
  const REFRESH_OPTS: { value: number; label: string }[] = [60, 72, 80, 90, 120].map((hz) => ({
    value: hz,
    label: `${hz} Hz`,
  }));

  const controlsDisabled = $derived(
    !configStore.view || configStore.loading || configStore.view.parseError != null || !configStore.view.exists,
  );

  function openSetupGate() {
    stageStore.openGate({
      stage: "setup",
      bottle: settingsStore.settings?.defaultBottle ?? null,
      bsDir: settingsStore.settings?.defaultBsDir ?? null,
      onFinished: () => {
        void configStore.load().then(resetDraft);
      },
    });
  }

  // ── save flow (write-once acknowledgement + confirm) ────────────────────────

  let showAckPanel = $state(false);
  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let lastWriteReport = $state<WriteReport | null>(null);

  function requestSave() {
    if (!isDirty || saving) return;
    lastWriteReport = null;
    saveError = null;
    if (!settingsStore.settings?.runtimeConfigEditAcknowledged) {
      showAckPanel = true;
      return;
    }
    void doSave();
  }

  async function confirmAck() {
    showAckPanel = false;
    try {
      await settingsStore.update({ runtimeConfigEditAcknowledged: true });
    } catch {
      // The gate is a courtesy, not a hard precondition — still attempt the write.
    }
    await doSave();
  }

  async function doSave() {
    saving = true;
    saveError = null;
    const toWrite = patch;
    try {
      lastWriteReport = await configStore.write(toWrite);
      resetDraft();
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function revertDraft() {
    resetDraft();
    saveError = null;
    lastWriteReport = null;
  }

  // ── repository card ──────────────────────────────────────────────────────

  let repoInfo = $state<RepoInfo | null>(null);
  let repoInfoLoading = $state(false);
  let repoInfoError = $state<string | null>(null);
  let checkoutBusy = $state(false);
  let checkoutError = $state<string | null>(null);
  let revertRoot = $state<string | null>(null);
  let hasRevertRoot = $state(false);

  async function loadRepoInfo() {
    repoInfoLoading = true;
    repoInfoError = null;
    try {
      repoInfo = await getRepoInfo();
    } catch (e) {
      repoInfoError = e instanceof Error ? e.message : String(e);
    } finally {
      repoInfoLoading = false;
    }
  }

  async function changeCheckout() {
    // Same rejection path as browseDefaultBsDir: surface it instead of dropping it.
    let dir: string | null;
    checkoutError = null;
    try {
      dir = await pickFolder("Choose the wine-vr checkout", settingsStore.settings?.repoRoot ?? null);
    } catch (e) {
      checkoutError = `Could not open the folder picker: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    if (!dir) return;
    checkoutBusy = true;
    const previous = settingsStore.settings?.repoRoot ?? null;
    try {
      await settingsStore.update({ repoRoot: dir });
      await loadRepoInfo();
      if (repoInfo && !repoInfo.markersPresent) {
        checkoutError = `"${dir}" doesn't look like a wine-vr checkout — missing demo.sh / scripts/demo/lib.sh.`;
        revertRoot = previous;
        hasRevertRoot = true;
      } else {
        hasRevertRoot = false;
      }
    } catch (e) {
      checkoutError = e instanceof Error ? e.message : String(e);
    } finally {
      checkoutBusy = false;
    }
  }

  async function revertCheckout() {
    checkoutBusy = true;
    try {
      await settingsStore.update({ repoRoot: revertRoot });
      await loadRepoInfo();
      checkoutError = null;
      hasRevertRoot = false;
    } catch (e) {
      checkoutError = e instanceof Error ? e.message : String(e);
    } finally {
      checkoutBusy = false;
    }
  }

  // ── footer: equivalent demo.sh command ──────────────────────────────────────

  const footerLaunchOpts = $derived.by(
    (): LaunchOpts => ({
      bottle: settingsStore.settings?.defaultBottle ?? null,
      bsDir: settingsStore.settings?.defaultBsDir ?? null,
      noAudio: settingsStore.settings?.launch.noAudio ?? false,
      noDashboard: settingsStore.settings?.launch.noDashboard ?? false,
      wired: settingsStore.settings?.launch.wired ?? false,
      verbose: settingsStore.settings?.launch.verbose ?? false,
    }),
  );
  const footerCommand = $derived(demoRunCommand(footerLaunchOpts));

  onMount(async () => {
    await Promise.all([settingsStore.load(), configStore.load()]);
    seedFromSettings();
    resetDraft();
    try {
      const state = await getAppState();
      bottles = state.bottles;
    } catch {
      bottles = [];
    } finally {
      bottlesLoaded = true;
    }
    void loadRepoInfo();
  });
</script>

<div class="screen-header">
  <h3>Settings</h3>
  {#if savedFlash}<span class="text-muted saved-badge">Saved</span>{/if}
</div>

<div class="screen-body">
  {#if settingsStore.error}
    <div class="banner">Could not load settings: {settingsStore.error}</div>
  {/if}
  {#if configStore.error}
    <div class="banner">Could not load the runtime config: {configStore.error}</div>
  {/if}
  {#if settingsSaveError}
    <div class="banner">Could not save: {settingsSaveError}</div>
  {/if}

  <div class="settings-grid">
    <div class="settings-col">
      <div class="blueprint card-panel">
        <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
        <div class="card-kicker">Streaming</div>
        <h5 class="card-panel-title">oxrsys-runtime.toml</h5>

        {#if !configStore.view || configStore.loading}
          <p class="text-muted">Loading…</p>
        {:else if configStore.view.parseError}
          <div class="parse-error">
            <p>Could not parse this file — Sabrage won't touch it until it's fixed by hand.</p>
            <p class="text-muted mono">{configStore.view.path}</p>
            <p class="text-muted">{configStore.view.parseError}</p>
          </div>
        {:else if !configStore.view.exists}
          <div class="setup-needed">
            <p class="text-muted">oxrsys-runtime.toml hasn't been created yet — run setup first.</p>
            <button class="btn btn-primary" onclick={openSetupGate}>Run setup…</button>
          </div>
        {:else}
          {#if isLegacyProtocol}
            <div class="legacy-banner">
              <strong>Sabrage will not launch with protocol = "oxrsys".</strong> The legacy USB-only path (adb
              reverse tunnels) is not implemented on this side: every launch from this app stops at the
              pre-launch gate with "Sabrage does not launch the legacy protocol — use ./demo.sh run --bottle
              &lt;name&gt;". Pick alvr here (or run Doctor's "Set protocol = alvr" fix) to launch from Sabrage. Audio
              routing / the dashboard toggle don't apply either (greyed out under Audio &amp; launch).
            </div>
          {/if}

          {#if configStore.view.invalid.length > 0}
            <div class="warn-block">
              {#each configStore.view.invalid as iv (iv.key)}
                <div class="warn-line">
                  <span class="mono">{iv.key}</span> = <span class="mono">"{iv.raw}"</span> — {iv.reason} (the
                  runtime silently ignores it and falls back to its own default)
                </div>
              {/each}
            </div>
          {/if}
          {#if configStore.view.shadowed.length > 0}
            <div class="warn-block">
              <div class="warn-line">
                Duplicate keys (the last occurrence in the file wins): {configStore.view.shadowed.join(", ")}
              </div>
            </div>
          {/if}

          <div class="field">
            <label for="settings-bitrate">Base bitrate — {effectiveBitrate} Mbps</label>
            <input
              id="settings-bitrate"
              type="range"
              min="10"
              max="150"
              step="1"
              value={effectiveBitrate}
              oninput={(e) => (draft = { ...draft, bitrateMbps: Number(e.currentTarget.value) })}
              disabled={controlsDisabled}
            />
            {#if draft.bitrateMbps === null}
              <div class="text-muted default-note">runtime default: {configStore.view.defaults.bitrateMbps}</div>
            {/if}
            <div class="text-muted field-note">
              Template writes 42; measured sessions ran at 80 (~5–6 dropped frames/s at the cap — tuning open).
              ALVR's adaptive loop adjusts from the base. The runtime accepts 1–200.
            </div>
          </div>

          <div class="field">
            <label for="settings-codec">Video codec</label>
            <div class="seg" id="settings-codec">
              {#each CODEC_OPTS as o (o.value)}
                <label class="seg-opt">
                  <input
                    type="radio"
                    name="settings-codec"
                    checked={effectiveVideoCodec === o.value}
                    onchange={() => (draft = { ...draft, videoCodec: o.value })}
                    disabled={controlsDisabled}
                  />
                  {o.label}
                </label>
              {/each}
            </div>
            {#if draft.videoCodec === null}
              <div class="text-muted default-note">runtime default: {configStore.view.defaults.videoCodec}</div>
            {/if}
            <div class="text-muted field-note">
              Honored for real on the helper path; the in-process fallback is H.264-only.
            </div>
          </div>

          <div class="field">
            <label for="settings-enc">Encoder process</label>
            <div class="seg" id="settings-enc">
              {#each ENCODER_OPTS as o (o.value)}
                <label class="seg-opt">
                  <input
                    type="radio"
                    name="settings-enc"
                    checked={effectiveEncoderProcess === o.value}
                    onchange={() => (draft = { ...draft, encoderProcess: o.value })}
                    disabled={controlsDisabled}
                  />
                  {o.label}
                </label>
              {/each}
            </div>
            {#if draft.encoderProcess === null}
              <div class="text-muted default-note">runtime default: {configStore.view.defaults.encoderProcess}</div>
            {/if}
            <div class="text-muted field-note">
              auto / native — out-of-process arm64 helper, HW HEVC (hard-required at launch). inproc — Rosetta
              H.264 fallback, macOS 27+.
            </div>
          </div>

          <div class="field">
            <label for="settings-proto">Streaming protocol</label>
            <div class="seg" id="settings-proto">
              {#each PROTOCOL_OPTS as o (o.value)}
                <label class="seg-opt">
                  <input
                    type="radio"
                    name="settings-proto"
                    checked={effectiveProtocol === o.value}
                    onchange={() => (draft = { ...draft, protocol: o.value })}
                    disabled={controlsDisabled}
                  />
                  {o.label}
                </label>
              {/each}
            </div>
            {#if draft.protocol === null}
              <div class="text-muted default-note">runtime default: {configStore.view.defaults.protocol}</div>
            {/if}
            <div class="text-muted field-note">
              alvr = WiFi/USB, the supported path. oxrsys = legacy USB-only client (adb reverse tunnels), which
              Sabrage refuses to launch — its pre-launch gate blocks it, and
              <span class="mono">./demo.sh run</span> is the only way to use it.
            </div>
          </div>

          <div class="field">
            <label for="settings-scale">
              Resolution scale — {effectiveResolutionScale.toFixed(2)}× ({resPx})
            </label>
            <input
              id="settings-scale"
              type="range"
              min="0.25"
              max="1"
              step="0.05"
              value={effectiveResolutionScale}
              oninput={(e) =>
                (draft = { ...draft, resolutionScale: Math.round(Number(e.currentTarget.value) * 100) / 100 })}
              disabled={controlsDisabled}
            />
            {#if draft.resolutionScale === null}
              <div class="text-muted default-note">
                runtime default: {configStore.view.defaults.resolutionScale}
              </div>
            {/if}
            <div class="text-muted field-note">Encode-side downscale; 1.0 = native. 3008×1664 @ 1.0 verified.</div>
          </div>

          <div class="field">
            <label for="settings-hz">Refresh rate</label>
            <div class="seg" id="settings-hz">
              {#each REFRESH_OPTS as o (o.value)}
                <label class="seg-opt">
                  <input
                    type="radio"
                    name="settings-hz"
                    checked={effectiveRefreshHz === o.value}
                    onchange={() => (draft = { ...draft, refreshRateHz: o.value })}
                    disabled={controlsDisabled}
                  />
                  {o.label}
                </label>
              {/each}
            </div>
            {#if draft.refreshRateHz === null}
              <div class="text-muted default-note">runtime default: {configStore.view.defaults.refreshRateHz}</div>
            {/if}
            <div class="text-muted field-note">72 Hz verified.</div>
          </div>

          <div class="hr"></div>

          {#if showAckPanel}
            <div class="ack-panel">
              <p>
                demo.sh treats this file as write-once; Sabrage edits values in place and keeps the last 10
                backups in <span class="mono">~/Library/Application Support/Sabrage/backups/</span>.
              </p>
              <div class="ack-actions">
                <button class="btn btn-secondary" onclick={() => (showAckPanel = false)}>Cancel</button>
                <button class="btn btn-primary" onclick={confirmAck}>Confirm &amp; save</button>
              </div>
            </div>
          {/if}

          {#if saveError}
            <div class="save-error">Save failed: {saveError}</div>
          {/if}

          {#if lastWriteReport}
            <div class="write-report">
              <div>
                Saved — changed:
                {lastWriteReport.changedKeys.length ? lastWriteReport.changedKeys.join(", ") : "nothing"}
              </div>
              {#if lastWriteReport.backupPath}
                <div class="text-muted mono">backup: {lastWriteReport.backupPath}</div>
              {/if}
              {#if lastWriteReport.createdFromTemplate}
                <div class="text-muted">created from the shared template (first write)</div>
              {/if}
              {#if lastWriteReport.shadowed.length}
                <div class="text-muted">resolved duplicate keys: {lastWriteReport.shadowed.join(", ")}</div>
              {/if}
            </div>
          {/if}

          <div class="streaming-actions">
            <span class="text-muted">Values take effect at the next launch.</span>
            <div class="streaming-actions-btns">
              <button class="btn btn-secondary" onclick={revertDraft} disabled={!isDirty || saving}>Revert</button>
              <button class="btn btn-primary" onclick={requestSave} disabled={!isDirty || saving}>
                {saving ? "Saving…" : "Save"}
              </button>
            </div>
          </div>
        {/if}
      </div>
    </div>

    <div class="settings-col">
      <div class="blueprint card-panel">
        <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
        <div class="card-kicker">Audio &amp; launch</div>
        <h5 class="card-panel-title">Defaults for every launch</h5>

        <div class="toggles">
          <label class="radio toggle-row">
            <input
              type="checkbox"
              bind:checked={routeAudioChk}
              onchange={onRouteAudioChange}
              disabled={!settingsStore.loaded || isLegacyProtocol}
            />
            <span class="dot toggle-dot"></span>
            <span class="toggle-body">
              <span class="toggle-title">Route audio to the headset</span>
              <span class="text-muted toggle-desc">
                All Mac output → BlackHole 2ch while a game runs; device + volume restored on exit.
              </span>
            </span>
          </label>
          <label class="radio toggle-row">
            <input
              type="checkbox"
              bind:checked={openDashboardChk}
              onchange={onOpenDashboardChange}
              disabled={!settingsStore.loaded || isLegacyProtocol}
            />
            <span class="dot toggle-dot"></span>
            <span class="toggle-body">
              <span class="toggle-title">Open the ALVR server dashboard on launch</span>
              <span class="text-muted toggle-desc">Polls 127.0.0.1:8082 inside the game process until it appears.</span>
            </span>
          </label>
          <label class="radio toggle-row">
            <input type="checkbox" bind:checked={wiredChk} onchange={onWiredChange} disabled={!settingsStore.loaded} />
            <span class="dot toggle-dot"></span>
            <span class="toggle-body">
              <span class="toggle-title">Wired (USB) streaming</span>
              <span class="text-muted toggle-desc">
                adb forward tcp:9943/9944 — a normal run clears the forwards (left behind they break WiFi
                discovery).
              </span>
            </span>
          </label>
          <label class="radio toggle-row">
            <input
              type="checkbox"
              bind:checked={verboseChk}
              onchange={onVerboseChange}
              disabled={!settingsStore.loaded}
            />
            <span class="dot toggle-dot"></span>
            <span class="toggle-body">
              <span class="toggle-title">Verbose wine/openxr debug channels</span>
              <span class="text-muted toggle-desc">Console stays quiet by default; this restores the firehose into the log.</span>
            </span>
          </label>
          {#if verboseChk}
            <div class="winedebug-note text-muted">
              WINEDEBUG=fixme-all,+openxr while verbose is on (default: WINEDEBUG=-all) — a caller-set WINEDEBUG
              env var always wins. Not independently editable in v1.
            </div>
          {/if}
          <label class="radio toggle-row">
            <input
              type="checkbox"
              bind:checked={allowAdbChk}
              onchange={onAllowAdbChange}
              disabled={!settingsStore.loaded}
            />
            <span class="dot toggle-dot"></span>
            <span class="toggle-body">
              <span class="toggle-title">Probe adb in Doctor</span>
              <span class="text-muted toggle-desc">Lets doctor checks shell out to adb (device/forward checks).</span>
            </span>
          </label>
        </div>
      </div>

      <div class="blueprint card-panel">
        <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
        <div class="card-kicker">Paths</div>
        <h5 class="card-panel-title">Bottle &amp; install directory</h5>

        <div class="field">
          <label for="settings-bottle">Default CrossOver bottle</label>
          {#if bottlesLoaded && bottles.length === 0}
            <span class="text-muted">none found — create one in the CrossOver UI</span>
          {:else}
            <select
              id="settings-bottle"
              class="input"
              bind:value={defaultBottleSel}
              onchange={onDefaultBottleChange}
              disabled={!settingsStore.loaded}
            >
              <option value="">— none —</option>
              {#each bottles as b (b)}
                <option value={b}>{b}</option>
              {/each}
            </select>
          {/if}
        </div>

        <div class="field">
          <label for="settings-bsdir">Default Beat Saber 1.29.4 install dir</label>
          <div class="browse-row">
            <input
              id="settings-bsdir"
              class="input mono"
              type="text"
              placeholder="leave empty to derive from the bottle"
              bind:value={bsDirInput}
              onchange={onBsDirCommit}
              disabled={!settingsStore.loaded}
            />
            <button class="btn btn-secondary" onclick={browseDefaultBsDir} disabled={!settingsStore.loaded}>
              Browse…
            </button>
          </div>
        </div>

        <div class="text-muted mono path-note">
          {configStore.view?.path ?? "~/Library/Application Support/OXRSys/oxrsys-runtime.toml"} — created once by
          setup; edited in place from the Streaming card above (with your confirmation).
        </div>
      </div>

      <div class="blueprint card-panel">
        <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
        <div class="card-kicker">Repository</div>
        <h5 class="card-panel-title">wine-vr checkout</h5>

        {#if repoInfoLoading && !repoInfo}
          <p class="text-muted">Loading…</p>
        {:else if repoInfoError}
          <p class="text-muted">{repoInfoError}</p>
        {:else if repoInfo}
          <div class="repo-rows">
            <div class="repo-row">
              <span class="repo-label">Repo root</span>
              <span class="mono">{repoInfo.repoRoot ?? "unresolved"}</span>
            </div>
            <div class="repo-row">
              <span class="repo-label">Source</span>
              <span>{repoInfo.source}</span>
            </div>
            <div class="repo-row">
              <span class="repo-label">Markers</span>
              <span class="tag {repoInfo.markersPresent ? 'tag-accent' : 'tag-outline'}">
                {repoInfo.markersPresent ? "present" : "missing"}
              </span>
            </div>
            <div class="repo-row">
              <span class="repo-label">Contract sync</span>
              <span
                class="tag {repoInfo.contractSynced === true
                  ? 'tag-accent'
                  : repoInfo.contractSynced === false
                    ? 'tag-outline'
                    : 'tag-neutral'}"
              >
                {repoInfo.contractSynced === true ? "synced" : repoInfo.contractSynced === false ? "stale" : "unknown"}
              </span>
            </div>
            <div class="repo-row">
              <span class="repo-label">Host manifest</span>
              <span class="mono">{repoInfo.hostManifestLibraryPath ?? "not written"}</span>
              <span
                class="tag {repoInfo.hostManifestPointsHere === true
                  ? 'tag-accent'
                  : repoInfo.hostManifestPointsHere === false
                    ? 'tag-outline'
                    : 'tag-neutral'}"
              >
                {repoInfo.hostManifestPointsHere === true
                  ? "matches"
                  : repoInfo.hostManifestPointsHere === false
                    ? "does not match"
                    : "unknown"}
              </span>
            </div>
          </div>
        {/if}

        <div class="repo-actions">
          <button class="btn btn-secondary" onclick={changeCheckout} disabled={checkoutBusy}>
            Change checkout…
          </button>
          {#if hasRevertRoot}
            <button class="btn btn-ghost" onclick={revertCheckout} disabled={checkoutBusy}>Revert</button>
          {/if}
        </div>
        {#if checkoutError}<div class="checkout-error">{checkoutError}</div>{/if}
      </div>
    </div>
  </div>

  <div class="footer-cmd">
    <span class="text-muted footer-cmd-label">Equivalent demo.sh flags:</span>
    <code class="cmd-text">{footerCommand}</code>
  </div>
</div>

<style>
  .screen-header {
    padding: 22px 28px 14px;
    border-bottom: 1px solid var(--color-divider);
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .screen-header h3 {
    margin: 0;
  }
  .saved-badge {
    font-size: 11.5px;
  }
  .screen-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 20px 28px 28px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .banner {
    border: 1px solid var(--color-divider);
    background: color-mix(in srgb, var(--color-accent-900) 10%, transparent);
    padding: 8px 14px;
    font-size: 12.5px;
  }
  .settings-grid {
    display: grid;
    grid-template-columns: minmax(320px, 1fr) minmax(300px, 1fr);
    gap: 20px;
    align-items: start;
  }
  @media (max-width: 900px) {
    .settings-grid {
      grid-template-columns: 1fr;
    }
  }
  .settings-col {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .card-panel {
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .card-panel-title {
    margin: 0 0 8px;
  }
  .field {
    margin-top: 12px;
  }
  .field:first-of-type {
    margin-top: 6px;
  }
  .field-note {
    font-size: 11px;
    margin-top: 4px;
  }
  .default-note {
    font-size: 11px;
    margin-top: 3px;
  }
  .mono {
    font-family: ui-monospace, Menlo, monospace;
  }
  .parse-error,
  .setup-needed {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 0;
  }
  .setup-needed button {
    align-self: flex-start;
  }
  .legacy-banner {
    border: 1px solid var(--color-accent-900);
    border-left-width: 3px;
    background: color-mix(in srgb, var(--color-accent-900) 14%, transparent);
    color: var(--color-accent-900);
    padding: 7px 10px;
    font-size: 12px;
    margin: 4px 0 6px;
  }
  .warn-block {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 4px 0;
  }
  .warn-line {
    font-size: 11.5px;
    color: var(--color-accent-900);
  }
  .ack-panel {
    border: 1px solid var(--color-divider);
    background: var(--color-surface);
    padding: 12px 14px;
    margin: 8px 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ack-panel p {
    margin: 0;
    font-size: 12.5px;
  }
  .ack-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .save-error {
    font-size: 11.5px;
    color: var(--color-accent-900);
    margin: 4px 0;
  }
  .write-report {
    border: 1px solid var(--color-divider);
    background: color-mix(in srgb, var(--color-accent) 7%, transparent);
    padding: 8px 12px;
    margin: 6px 0;
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .streaming-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 10px;
    font-size: 11.5px;
  }
  .streaming-actions-btns {
    display: flex;
    gap: 8px;
  }
  .toggles {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 6px;
  }
  .toggle-row {
    align-items: flex-start;
  }
  .toggle-dot {
    border-radius: 0 !important;
    margin-top: 2px;
  }
  .toggle-body {
    display: flex;
    flex-direction: column;
  }
  .toggle-title {
    font-size: 13.5px;
  }
  .toggle-desc {
    font-size: 11px;
  }
  .winedebug-note {
    font-size: 11px;
    margin-left: 26px;
    margin-top: -4px;
  }
  .browse-row {
    display: flex;
    gap: 8px;
  }
  .browse-row .input {
    flex: 1;
    min-width: 0;
  }
  .path-note {
    font-size: 11px;
    margin-top: 10px;
  }
  .repo-rows {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 6px;
  }
  .repo-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 12.5px;
    flex-wrap: wrap;
  }
  .repo-label {
    flex: none;
    width: 96px;
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-text) 55%, transparent);
  }
  .repo-actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
  .checkout-error {
    font-size: 11.5px;
    color: var(--color-accent-900);
    margin-top: 6px;
  }
  .footer-cmd {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .footer-cmd-label {
    font-size: 11.5px;
    flex: none;
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
</style>
