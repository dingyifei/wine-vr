// Hand-mirrored IPC boundary between the Svelte frontend and sabrage-app's
// Tauri commands (`src-tauri/src/commands.rs`). No codegen — when either side
// changes a shape, update both by hand and keep this comment as the pointer.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

/** Mirrors `sabrage_core::checks::CheckStatus` (serde `rename_all = "snake_case"`). */
export type CheckStatus = "pass" | "warn" | "fail" | "info" | "skipped" | "not_implemented";

/** One streamed doctor row — mirrors `commands::DoctorEvent` (serde camelCase). */
export interface DoctorEvent {
  slug: string;
  group: string;
  status: CheckStatus;
  message: string;
  remedy: string | null;
  detail: string | null;
  /** Bare contract fix id (e.g. `"fix.set-graphics-backend"`), or `null` when
   * this check's remedy has none. Resolve to a `FixAction` with
   * `contractFixIdToAction` before offering a Fix button — `fix.create-z-drive`
   * is the one contract id deliberately left unmodelled and resolves to
   * `null` (`fix.edit-protocol` gained a `FixAction` in Phase 4). */
  fix: string | null;
}

/** The aggregate `run_doctor` resolves to — mirrors `commands::DoctorSummary`. */
export interface DoctorSummary {
  failCount: number;
  warnCount: number;
  total: number;
}

/** Sidebar footer snapshot — mirrors `commands::AppState`. `defaultBottle`/
 * `defaultBsDir` are Phase 4 additions (from `Settings`) so the Sidebar and
 * Session screen can prefill without a second `getSettings()` call. */
export interface AppState {
  repoRoot: string | null;
  bottles: string[];
  alvrVersion: string;
  defaultBottle: string | null;
  defaultBsDir: string | null;
}

export interface RunDoctorArgs {
  bottle?: string | null;
  bsDir?: string | null;
}

/**
 * Run every doctor check, streaming each resolved row to `onEvent` in contract
 * order as it settles, then resolving to the aggregate.
 *
 * Rejects if the wine-vr repo root cannot be resolved (`SABRAGE_REPO_ROOT`
 * unset and no checkout found above the running executable) — the backend
 * also streams that failure as a single synthetic `meta.repo-root` row first,
 * so a caller that only watches `onEvent` still learns why nothing ran.
 */
export async function runDoctor(
  args: RunDoctorArgs,
  onEvent: (event: DoctorEvent) => void,
): Promise<DoctorSummary> {
  const channel = new Channel<DoctorEvent>();
  channel.onmessage = onEvent;
  return invoke<DoctorSummary>("run_doctor", {
    bottle: args.bottle ?? null,
    bsDir: args.bsDir ?? null,
    onEvent: channel,
  });
}

/** Fetch the sidebar footer snapshot (repo root, bottles, pinned ALVR version). */
export async function getAppState(): Promise<AppState> {
  return invoke<AppState>("get_app_state");
}

// ── pipeline stages + fixes ─────────────────────────────────────────────────
//
// Mirrors `sabrage_core::events` / `sabrage_core::fixes` (via
// `src-tauri/src/commands.rs`, which forwards `StageEvent` to the channel
// verbatim — there is no separate wire type on the Rust side either).

/** Mirrors `sabrage_core::Stage` (serde lowercase). `run_stage(stage: "run")`
 * works as of Phase 3, but its promise does not resolve until the session
 * ends — see `launch()`'s own doc comment; the Session screen calls `launch`,
 * not `runStage`, for that reason. */
export type Stage = "setup" | "build" | "install" | "run" | "stop";

/** Mirrors `sabrage_core::Severity` (serde camelCase — all four words are
 * already lowercase, so this looks like plain lowercase). */
export type Severity = "info" | "ok" | "warn" | "fail";

/** Mirrors `sabrage_core::Stream` (serde camelCase). Named `StreamKind` here —
 * `Stream` collides with the DOM type. */
export type StreamKind = "stdout" | "stderr";
export type ChunkEnd = "lf" | "cr" | "eof";

/** Mirrors `sabrage_core::FixAction` (serde kebab-case — the contract id
 * without its `fix.` prefix). */
export type FixAction =
  | "set-graphics-backend"
  | "restage-helper"
  | "remove-adb-forwards"
  | "delete-session-json"
  | "run-setup"
  | "run-build"
  | "run-install"
  | "edit-protocol";

/** One `sabrage_core::StageEvent` variant — internally tagged on `kind`,
 * camelCase fields (events.rs). Forwarded to the channel verbatim by every
 * stage/fix command, so this is the wire shape, not a projection of it.
 * `Line.text` / `Fatal.message` are the shell's own strings verbatim — no
 * marker, no colour, no leading spaces; render those. */
export type StageEvent =
  | { kind: "stageStarted"; runId: string; stage: Stage }
  | { kind: "section"; runId: string; title: string }
  | {
      kind: "line";
      runId: string;
      step: string | null;
      severity: Severity;
      text: string;
      remedy: string | null;
    }
  | {
      kind: "output";
      runId: string;
      step: string;
      stream: StreamKind;
      chunk: string;
      /** How the chunk ended (`process::ChunkEnd`, camelCase): `"lf"` a line,
       * `"cr"` a bare `\r` repaint of the same terminal line, `"eof"` the
       * stream's unterminated tail. Optional on the wire (`#[serde(default)]`,
       * default `"lf"`) so an older core is still readable. */
      end?: ChunkEnd;
    }
  | {
      kind: "progress";
      runId: string;
      step: string;
      label: string;
      current: number;
      total: number | null;
    }
  | { kind: "autoFixed"; runId: string; step: string; fix: FixAction; description: string }
  | { kind: "needsAdmin"; runId: string; step: string; reason: string }
  | { kind: "fatal"; runId: string; message: string; remedy: string | null; fix: FixAction | null }
  // ── Phase 3 (run) ──
  /** A raw `print -r --` line, reproduced verbatim — leading spaces and all,
   * possibly empty. `run.sh`'s launch banner and its audio/dashboard/exit
   * lines are the only source of these (see `events.rs`'s `StageEvent::Text`
   * doc comment for the full verbatim list). */
  | { kind: "text"; runId: string; step: string | null; text: string }
  /** One launch-preflight check's final outcome. `gate` is the contract's
   * `native_gate` for the slug — "this warn is advisory" vs. "this warn would
   * have blocked on the zsh side" without re-reading the contract. */
  | { kind: "check"; runId: string; step: string; outcome: CheckOutcome; gate: Gate }
  /** The wine child is up — the run has left preflight/prepare and is now a
   * session. `pid`/`startTime` together are the process identity persisted
   * into `session-state.json`; `startedAtUnixMs` is wall-clock, for the
   * session timer. */
  | {
      kind: "launched";
      runId: string;
      pid: number;
      startTime: number;
      logPath: string;
      startedAtUnixMs: number;
    }
  | { kind: "stageFinished"; runId: string; stage: Stage; ok: boolean; exitCodeEquiv: number };

/** The launch preflight's per-side treatment of a check's failure — mirrors
 * `sabrage_core::contract::Gate` (serde lowercase). */
export type Gate = "block" | "warn" | "autofix" | "none";

/** One resolved check outcome — mirrors `sabrage_core::checks::CheckOutcome`'s
 * serialized fields (`quiet` is `#[serde(skip)]`, never on the wire). Distinct
 * from `DoctorEvent` above: this is the bare struct `StageEvent::Check` embeds
 * (no `group`/`fix` — those are the doctor-screen projection `run_doctor`
 * layers on top via the contract). */
export interface CheckOutcome {
  slug: string;
  status: CheckStatus;
  message: string;
  remedy: string | null;
  detail: string | null;
}

/** Mirrors `sabrage_core::StageOutcome`. */
export interface StageOutcome {
  stage: Stage;
  ok: boolean;
  exitCodeEquiv: number;
}

/** Mirrors `sabrage_core::FixReport`. */
export interface FixReport {
  action: FixAction;
  changed: boolean;
  description: string;
}

export interface StageRunOpts {
  bottle?: string | null;
  bsDir?: string | null;
  dryRun?: boolean;
}

export interface FixRunOpts {
  bottle?: string | null;
  bsDir?: string | null;
}

/** Static metadata about each fix — hand-mirrors `sabrage_core::fixes::fix_defs()`
 * plus `FixAction::as_stage()`. Only `runInstall` needs admin; only
 * `deleteSessionJson` is destructive (and carries a `consequence` the confirm
 * dialog must show); `stage` names the whole-stage fixes whose intended UI
 * path is `runStage` directly (see `contractFixIdToAction`'s doc comment)
 * rather than `applyFix`.
 *
 * Every fix is additionally `forbidden_while_session_live` on the Rust side
 * and refused by `fixes::apply` before it takes the operation lock, so a Fix
 * button offered during a live session fails with a remedy rather than
 * mutating; disable them with `blocksMutation(status.phase)` so the user
 * learns that before clicking. */
export const FIX_META: Record<
  FixAction,
  {
    title: string;
    needsAdmin: boolean;
    destructive: boolean;
    stage: Stage | null;
    /** What the user must know *before* confirming, for a remedy whose known
     * outcome is worse than the row it fixes (`fixes::FixDef::consequence`).
     * A confirm dialog must render this **instead of** a generic "this cannot
     * be undone" — the disclosure belongs before the mutation, not in the
     * report after it. `null` for every ordinary fix. */
    consequence: string | null;
  }
> = {
  "set-graphics-backend": {
    title: "force the bottle's graphics backend to dxmt",
    needsAdmin: false,
    destructive: false,
    stage: null,
    consequence: null,
  },
  "restage-helper": {
    title: "restage the arm64 encoder helper",
    needsAdmin: false,
    destructive: false,
    stage: null,
    consequence: null,
  },
  "remove-adb-forwards": {
    title: "remove the stale adb port forwards",
    needsAdmin: false,
    destructive: false,
    stage: null,
    consequence: null,
  },
  "delete-session-json": {
    title: "delete ALVR's session.json (clears pinned client IPs)",
    needsAdmin: false,
    destructive: true,
    stage: null,
    consequence:
      "Known-bad remedy: deleting this file has been observed to leave the client at " +
      "an 800x900 black screen. The file is copied to Application " +
      "Support/Sabrage/backups first, and editing the pinned IP in place is the " +
      "recovery that works.",
  },
  "run-setup": {
    title: "run setup",
    needsAdmin: false,
    destructive: false,
    stage: "setup",
    consequence: null,
  },
  "run-build": {
    title: "run build",
    needsAdmin: false,
    destructive: false,
    stage: "build",
    consequence: null,
  },
  "run-install": {
    title: "run install",
    needsAdmin: true,
    destructive: false,
    stage: "install",
    consequence: null,
  },
  "edit-protocol": {
    title: 'set protocol = "alvr" in oxrsys-runtime.toml',
    needsAdmin: false,
    destructive: false,
    stage: null,
    consequence: null,
  },
};

/** `"fix.set-graphics-backend"` -> `"set-graphics-backend"`; `null` for a
 * contract id this table does not model — mirrors
 * `FixAction::from_contract_id`. `fix.create-z-drive` is the one remaining
 * deliberately-deferred id (`fix.edit-protocol` resolves to
 * `"edit-protocol"` as of Phase 4). A `null` result means "render no Fix
 * button", not an error. */
export function contractFixIdToAction(id: string): FixAction | null {
  const bare = id.startsWith("fix.") ? id.slice(4) : id;
  return bare in FIX_META ? (bare as FixAction) : null;
}

/**
 * Run one pipeline stage, streaming every `StageEvent` to `onEvent` as it
 * happens. The returned promise does not resolve until the stage settles —
 * drive UI off `onEvent` (in particular `stageFinished`) and treat the
 * resolved/rejected promise as a secondary confirmation.
 */
export async function runStage(
  stage: Stage,
  opts: StageRunOpts,
  onEvent: (event: StageEvent) => void,
): Promise<StageOutcome> {
  const channel = new Channel<StageEvent>();
  channel.onmessage = onEvent;
  return invoke<StageOutcome>("run_stage", {
    stage,
    opts: {
      bottle: opts.bottle ?? null,
      bsDir: opts.bsDir ?? null,
      dryRun: opts.dryRun ?? null,
    },
    onEvent: channel,
  });
}

/** Cancel the run named `runId` (from its first `stageStarted` event), if
 * still in flight. Resolves `false` when no such run is tracked. */
export async function cancelStage(runId: string): Promise<boolean> {
  return invoke<boolean>("cancel_stage", { runId });
}

/**
 * Stop the session. As of Phase 3 this covers two cases (see
 * `commands::stop_session`'s doc comment):
 *
 * - A session this Sabrage process is supervising (`launch()` is still
 *   pending) is stopped by firing its own cancel token — `bottle` is ignored,
 *   and `onEvent` receives nothing new: the still-pending `launch()` call's
 *   own `onEvent` carries every teardown row. The resolved `StageOutcome` is
 *   synthetic (`{ stage: "run", ok: true, exitCodeEquiv: 130 }`, INT parity).
 * - Otherwise, runs the `stop` stage for `bottle` as before.
 *
 * `sessionStore.stop()` is the intended call site for both — it supplies
 * `bottle` from `sessionStore.status.bottle` so callers never need to track
 * one themselves.
 *
 * The live-session branch **rejects** when teardown did not actually finish:
 * it timed out (the session may still be running), or the run detached
 * instead of stopping. Show that message — it used to resolve `ok: true`
 * regardless, so "Stopped" could appear over a still-running game.
 */
export async function stopSession(
  bottle: string | null | undefined,
  onEvent: (event: StageEvent) => void,
): Promise<StageOutcome> {
  const channel = new Channel<StageEvent>();
  channel.onmessage = onEvent;
  return invoke<StageOutcome>("stop_session", { bottle: bottle ?? null, onEvent: channel });
}

/**
 * Apply one fix. Destructive fixes (`FIX_META[action].destructive`) must not
 * be called with `confirmed: false` — the backend refuses them (see
 * `commands::fix`'s doc comment); show an in-app confirm dialog first, never
 * `window.confirm` (it blocks the webview).
 */
export async function applyFix(
  action: FixAction,
  opts: FixRunOpts,
  confirmed: boolean,
  onEvent: (event: StageEvent) => void,
): Promise<FixReport> {
  const channel = new Channel<StageEvent>();
  channel.onmessage = onEvent;
  return invoke<FixReport>("fix", {
    action,
    opts: {
      bottle: opts.bottle ?? null,
      bsDir: opts.bsDir ?? null,
    },
    confirmed,
    onEvent: channel,
  });
}

// ── session / launch (Phase 3) ──────────────────────────────────────────────
//
// Mirrors `sabrage_core::session` (via `src-tauri/src/commands.rs`).

/** Mirrors `sabrage_core::session::SessionPhase` (serde camelCase — every
 * word is already lowercase, so this looks like plain lowercase).
 *
 * `"external"` is a session **nothing in Sabrage started** — no live handle,
 * no `session-state.json`, but a fresh `runtime_status.json` naming a live
 * pid, i.e. a `./demo.sh run` in another terminal. It is a live session in
 * every way that matters to this UI (see `isLivePhase`): reporting it as
 * `"idle"` invites a second launch over a running game. `stopSession()` can
 * target it — the `stop` stage works recordless. */
export type SessionPhase =
  | "idle"
  | "preflight"
  | "launching"
  | "running"
  | "stalled"
  | "stopping"
  | "exited"
  | "detached"
  | "external";

/** Is something running (or about to be) that a mutating action must not
 * disturb? The one definition of "a session is live" for the whole UI —
 * screens used to re-derive their own phase sets, and drifted.
 *
 * `"exited"` is not live (the wine child is gone, the row is just the last
 * session's epitaph) and neither is `"idle"`. Everything else is. */
export function isLivePhase(phase: SessionPhase): boolean {
  return phase !== "idle" && phase !== "exited";
}

/** Is the game actually up (as opposed to starting, stopping, or someone
 * else's)? `"stalled"` counts: the documented standby freeze is a running
 * session that stopped streaming. */
export function isActivePhase(phase: SessionPhase): boolean {
  return phase === "running" || phase === "stalled";
}

/** May Sabrage mutate the machine right now (setup/build/install, a Doctor
 * fix, a config write)? The frontend half of
 * `sabrage_core::session::ensure_idle` / `stages::live_session_block` — the
 * backend refuses these too, so this is for disabling buttons with an
 * explanation rather than for correctness. */
export function blocksMutation(phase: SessionPhase): boolean {
  return isLivePhase(phase);
}

/** Is there something for a Stop button to act on? Same set as `isLivePhase`
 * — `stopSession()` handles all of them (its own cancel token for a session
 * this process supervises, the bottle-scoped `stop` stage otherwise). */
export function canStop(phase: SessionPhase): boolean {
  return isLivePhase(phase);
}

/** One phase -> one semantic tone, so every screen's dot/pill/badge agrees.
 * `satisfies` (not `:`) so a new `SessionPhase` member is a type error here
 * rather than an `undefined` at runtime. */
export const PHASE_TONE = {
  idle: "muted",
  preflight: "warn",
  launching: "warn",
  running: "ok",
  stalled: "bad",
  stopping: "warn",
  exited: "muted",
  detached: "warn",
  external: "warn",
} satisfies Record<SessionPhase, "ok" | "warn" | "bad" | "muted">;

/** Mirrors `sabrage_core::session::EncoderInfo` — the most recent `encoder
 * ready …` log line, when one has been seen. `path` is `"native helper"` or
 * `"in-process"` verbatim; the latter is the silent H.264-Rosetta-downgrade
 * signature CLAUDE.md calls out. */
export interface EncoderInfo {
  codec: string;
  path: string;
  width: number;
  height: number;
  refreshHz: number;
  bitrateMbps: number;
}

/** Mirrors `sabrage_core::session::SessionStatus` — the `session://status`
 * broadcast payload and `getSessionStatus()`'s return. `runtimeState` is an
 * **opaque** string (the enum is unverified upstream); trust it only while
 * `runtimeFresh` is true. */
export interface SessionStatus {
  phase: SessionPhase;
  runId: string | null;
  bottle: string | null;
  pid: number | null;
  startedAtUnixMs: number | null;
  exitCode: number | null;
  logPath: string | null;
  encoder: EncoderInfo | null;
  runtimeState: string | null;
  runtimeFresh: boolean;
  ownedByThisProcess: boolean;
  detached: boolean;
}

/** The idle snapshot — mirrors `SessionStatus::default()` in Rust, for stores
 * to seed themselves with before the first real snapshot arrives. */
export const IDLE_SESSION_STATUS: SessionStatus = {
  phase: "idle",
  runId: null,
  bottle: null,
  pid: null,
  startedAtUnixMs: null,
  exitCode: null,
  logPath: null,
  encoder: null,
  runtimeState: null,
  runtimeFresh: false,
  ownedByThisProcess: false,
  detached: false,
};

/** Mirrors `commands::LaunchOpts` (serde camelCase). Every field but the
 * bottle/bs-dir pair has no `demo.sh` counterpart at all outside `run.sh`
 * itself — see that struct's own doc comment. `gameId` (Phase 4) is the
 * library entry this launch belongs to, when it came from the Library
 * screen's Run button — the backend uses it to record a `LastSession` after
 * the run settles; omit it for the plain Session screen's own launches. */
export interface LaunchOpts {
  bottle?: string | null;
  bsDir?: string | null;
  noAudio?: boolean | null;
  noDashboard?: boolean | null;
  wired?: boolean | null;
  verbose?: boolean | null;
  dryRun?: boolean | null;
  gameId?: string | null;
}

/**
 * Launch Beat Saber through the bridge (`Stage::Run`).
 *
 * **The returned promise does not resolve until the session ends** — routinely
 * hours. Drive UI off `onEvent` (`"launched"` marks "the game is up",
 * `"stageFinished"` marks "the session is over") and treat the
 * resolved/rejected promise as a secondary confirmation, exactly like every
 * other stage command here. `sessionStore.launch` is the intended call site —
 * it owns `launchRows`/`launching`/`lastOutcome`/`lastError` so more than one
 * component can observe the same in-flight launch.
 */
export async function launch(
  opts: LaunchOpts,
  onEvent: (event: StageEvent) => void,
): Promise<StageOutcome> {
  const channel = new Channel<StageEvent>();
  channel.onmessage = onEvent;
  return invoke<StageOutcome>("launch", {
    opts: {
      bottle: opts.bottle ?? null,
      bsDir: opts.bsDir ?? null,
      noAudio: opts.noAudio ?? null,
      noDashboard: opts.noDashboard ?? null,
      wired: opts.wired ?? null,
      verbose: opts.verbose ?? null,
      dryRun: opts.dryRun ?? null,
      gameId: opts.gameId ?? null,
    },
    onEvent: channel,
  });
}

/** One [`SessionStatus`] snapshot. Prefer `onSessionStatus`'s 1 Hz broadcast
 * for anything reactive; this is the poll-fallback / initial-value call. */
export async function getSessionStatus(): Promise<SessionStatus> {
  return invoke<SessionStatus>("get_session_status");
}

/** Detach from the live session — the app-quit "leave it running" answer
 * (critique.md, "app-quit semantics for a live session"). A no-op when
 * nothing is live. */
export async function detachSession(): Promise<void> {
  return invoke("detach_session");
}

// ── reconcile ────────────────────────────────────────────────────────────────

/** One `adb forward tcp:<port> tcp:<port>` a `--wired` launch created —
 * mirrors `sabrage_core::session::state::WiredForward`. */
export interface WiredForward {
  serial: string;
  port: number;
}

/** Which persisted guards a crash-recovery pass has already released —
 * mirrors `sabrage_core::session::state::GuardFlags`. */
export interface GuardFlags {
  audioRestored: boolean;
  dashboardClosed: boolean;
  forwardsCleared: boolean;
}

/** A resolved process identity (pid + start time, the pair that survives a
 * recycled pid) — mirrors `sabrage_core::process::ProcInfo`. */
export interface ProcInfo {
  pid: number;
  startTime: number;
  exe: string;
}

/** The on-disk crash-recovery record — mirrors
 * `sabrage_core::session::state::SessionState`. */
export interface SessionState {
  version: number;
  runId: string;
  bottle: string;
  bsDir: string;
  startedAtUnixMs: number;
  logPath: string;
  ownerPid: number;
  wine: ProcInfo | null;
  dashboard: ProcInfo | null;
  prevAudioOutput: string | null;
  wiredForwards: WiredForward[];
  guards: GuardFlags;
  detached: boolean;
}

/** Mirrors `sabrage_core::session::reconcile::Reconciled` (serde `tag:
 * "kind"`, camelCase). `restored` is one human line per guard a `dead`/
 * `identityMismatch` pass undid. */
export type Reconciled =
  | { kind: "noSession" }
  | { kind: "live"; state: SessionState }
  /** `pending` — a guard could not be released, so the record was kept rather
   * than cleared; the next launch must not overwrite it blind. */
  | { kind: "dead"; state: SessionState; restored: string[]; pending: boolean }
  | { kind: "identityMismatch"; state: SessionState; restored: string[]; pending: boolean }
  /** The record belongs to somebody — a launch in flight here, another live
   * front-end, or a newer Sabrage. Reported and nothing else: nothing
   * restored, nothing signalled, the file untouched. `reason` is the row the
   * user saw. */
  | { kind: "busy"; state: SessionState; reason: string };

/** `reconcileSession`'s return: the classification plus every human line
 * emitted while producing it (mirrors `commands::ReconcileReport` — the field
 * really is named `kind` and really does hold a `Reconciled`, whose own tag
 * field happens to share that name; see that Rust struct's doc comment). */
export interface ReconcileReport {
  kind: Reconciled;
  rows: string[];
}

/**
 * Reconcile whatever `session-state.json` says on disk against what is
 * actually running. Call at startup and again before showing the Launch
 * button (design-app §6). Request/response, not a stream — there is no
 * `onEvent` here.
 */
export async function reconcileSession(bottle: string | null): Promise<ReconcileReport> {
  return invoke<ReconcileReport>("reconcile_session", { bottle });
}

// ── logs ─────────────────────────────────────────────────────────────────────

/** Mirrors `sabrage_core::logs::LogSource` (serde `tag: "kind"`, camelCase).
 * `file`'s `path` is a specific past run from `listPastRuns()`. */
export type LogSource =
  | { kind: "wineConsole" }
  | { kind: "oxrsysRuntime" }
  | { kind: "alvrSession" }
  | { kind: "file"; path: string };

/** One tail poll's worth of new lines — mirrors `sabrage_core::logs::LogBatch`. */
export interface LogBatch {
  lines: string[];
  rotated: boolean;
  truncated: boolean;
  path: string;
}

/** One `logs/beatsaber-*.log` on disk — mirrors `sabrage_core::logs::PastRun`. */
export interface PastRun {
  path: string;
  fileName: string;
  size: number;
  modifiedUnixMs: number;
}

/**
 * Start tailing `source`, streaming each non-empty batch to `onBatch` until
 * `stopLogTail` is called with the returned id. Rejects when the source has
 * no file yet (an empty `logs/` on a fresh checkout, no session ever run).
 */
export async function startLogTail(
  source: LogSource,
  onBatch: (batch: LogBatch) => void,
): Promise<number> {
  const channel = new Channel<LogBatch>();
  channel.onmessage = onBatch;
  return invoke<number>("start_log_tail", { source, onBatch: channel });
}

/** Stop a tail started by `startLogTail`. Tails also stop themselves when
 * their task ends and are all stopped on a webview reload, so an id may
 * already be gone; that is not an error. */
export async function stopLogTail(id: number): Promise<void> {
  return invoke("stop_log_tail", { id });
}

/** Every `logs/beatsaber-*.log` on disk, newest first — both front-ends'
 * runs, since they share the directory. */
export async function listPastRuns(): Promise<PastRun[]> {
  return invoke<PastRun[]>("list_past_runs");
}

/** Resolve `source` to a path on this machine, or `null` when nothing
 * matches yet. */
export async function getLogSourcePath(source: LogSource): Promise<string | null> {
  return invoke<string | null>("get_log_source_path", { source });
}

// ── quit ─────────────────────────────────────────────────────────────────────

/** The three answers to "a session is still running — quit anyway?" — mirrors
 * `commands::QuitChoice` (serde camelCase; every variant is one already-lower
 * word). */
export type QuitChoice = "stop" | "keep" | "cancel";

/** Resolve the pending quit-while-live dialog (`onQuitRequested`). `"stop"`/
 * `"keep"` end by exiting the app from the Rust side — do not expect this
 * promise's *settlement* to be observable from the webview either way, only
 * `"cancel"` reliably returns control to the caller.
 *
 * `"stop"` rejects (while still exiting) when teardown did not finish inside
 * its 30 s budget: Sabrage then detaches instead — guards disarmed, the
 * record marked detached, the game left running — rather than exiting with a
 * half-torn-down session. A quit dialog left unanswered for 20 s is also
 * given up on: the next quit request is no longer intercepted at all and the
 * app exits through the same keep-running path, so a webview that died before
 * subscribing to `onQuitRequested` can never make Sabrage unquittable. */
export async function resolveQuit(choice: QuitChoice): Promise<void> {
  return invoke("resolve_quit", { choice });
}

// ── global events ────────────────────────────────────────────────────────────

/** Subscribe to the 1 Hz `session://status` broadcast. */
export async function onSessionStatus(cb: (status: SessionStatus) => void): Promise<UnlistenFn> {
  return listen<SessionStatus>("session://status", (event) => cb(event.payload));
}

/** A run that has an id and a cancellation handle but is still waiting for the
 * process-wide operation lock — mirrors `commands::QueuedStage`. */
export interface QueuedStage {
  runId: string;
  stage: Stage;
}

/**
 * Subscribe to `stage://queued` — fired when a `runStage`/`launch`/
 * `stopSession` call finds another operation already in flight and has to
 * wait for it.
 *
 * `run_stage` takes the operation lock *before* emitting its first
 * `stageStarted`, so until this event existed a queued run had no id the UI
 * could offer Cancel for: the button stayed disabled for the whole wait, and
 * then the stage mutated the machine when its turn came. Cancelling on this
 * id works — the executor fails every filesystem primitive once the token
 * fires, so a run cancelled while queued is a no-op when it finally gets the
 * lock. Treat it exactly like `stageStarted`'s `runId` (and expect
 * `stageStarted` for the same run later).
 */
export async function onStageQueued(cb: (queued: QueuedStage) => void): Promise<UnlistenFn> {
  return listen<QueuedStage>("stage://queued", (event) => cb(event.payload));
}

/** Subscribe to `app://quit-requested` — fired when `ExitRequested`/
 * `CloseRequested` was intercepted because a session is still live. Answer it
 * with `resolveQuit` — a dialog left unanswered past 20 s is given up on and
 * the next quit request exits the app (detaching from the session). */
export async function onQuitRequested(cb: () => void): Promise<UnlistenFn> {
  return listen("app://quit-requested", () => cb());
}

/** Subscribe to the Pipeline menu's three enabled items (`menu://doctor` /
 * `menu://launch` / `menu://stop`), unified into one callback. The returned
 * function unsubscribes all three. */
export async function onMenu(cb: (which: "doctor" | "launch" | "stop") => void): Promise<UnlistenFn> {
  const unlistens = await Promise.all([
    listen("menu://doctor", () => cb("doctor")),
    listen("menu://launch", () => cb("launch")),
    listen("menu://stop", () => cb("stop")),
  ]);
  return () => {
    for (const unlisten of unlistens) unlisten();
  };
}

// ── runtime config (Phase 4) ────────────────────────────────────────────────
//
// Mirrors `sabrage_core::config::runtime_toml` (via `src-tauri/src/commands.rs`).
// Edits `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` in place —
// the deliberate divergence from demo.sh's write-once treatment of that file
// (design-app.md §4, "Settings write policy").

/** Mirrors `config::runtime_toml::Protocol` (serde lowercase). */
export type Protocol = "alvr" | "oxrsys";

/** Mirrors `config::runtime_toml::VideoCodec` (serde lowercase). */
export type VideoCodec = "auto" | "h265" | "h264";

/** Mirrors `config::runtime_toml::EncoderProcess` (serde lowercase). */
export type EncoderProcess = "auto" | "native" | "inproc";

/** The six keys Sabrage edits in `[streaming]` — mirrors
 * `config::runtime_toml::RuntimeConfigValues` (serde camelCase). Every field
 * is `Option` on the Rust side: `null` here means "not present in the file"
 * on a `RuntimeConfigView.values`/`.defaults`, or "leave this key untouched"
 * on a `RuntimeConfigPatch`. */
export interface RuntimeConfigValues {
  protocol: Protocol | null;
  bitrateMbps: number | null;
  encoderProcess: EncoderProcess | null;
  videoCodec: VideoCodec | null;
  resolutionScale: number | null;
  refreshRateHz: number | null;
}

/** One key's on-disk value the runtime would silently ignore (outside its
 * accepted set) — mirrors `config::runtime_toml::InvalidValue`. */
export interface InvalidValue {
  key: string;
  raw: string;
  reason: string;
}

/** Mirrors `config::runtime_toml::RuntimeConfigView` — `read_runtime_config`'s
 * return shape. */
export interface RuntimeConfigView {
  path: string;
  exists: boolean;
  values: RuntimeConfigValues;
  /** The runtime's own compiled-in defaults, for rendering "runtime default:
   * <x>" next to a `null` value. */
  defaults: RuntimeConfigValues;
  invalid: InvalidValue[];
  /** Keys present more than once across tables (top level counts as one) —
   * the LAST occurrence in the file is the one the runtime honors and the
   * one reflected in `values`; render these as a warning. */
  shadowed: string[];
  modifiedUnixMs: number | null;
  /** Set when `toml_edit` could not parse the file at all: `values` then
   * comes from the line-oriented fallback reader, and `writeRuntimeConfig`
   * refuses to touch the file until this is fixed by hand. */
  parseError: string | null;
}

/** A patch to `oxrsys-runtime.toml`: a non-null field sets that key, `null`
 * leaves it untouched. Same shape as `RuntimeConfigValues` on the Rust side
 * too (`type RuntimeConfigPatch = RuntimeConfigValues`). */
export type RuntimeConfigPatch = RuntimeConfigValues;

/** Mirrors `config::runtime_toml::WriteReport` — `write_runtime_config`'s
 * return shape. */
export interface WriteReport {
  createdFromTemplate: boolean;
  /** Set whenever an existing file was overwritten — the snapshot lives
   * under `<Sabrage appsup>/backups/`, newest `BACKUP_KEEP` (10) kept. */
  backupPath: string | null;
  changedKeys: string[];
  shadowed: string[];
  path: string;
}

/** Read the current `oxrsys-runtime.toml` state. Never rejects for a missing
 * file (`exists: false`, every value `null`) — only a genuine IPC-layer
 * failure (repo root unresolved) throws. */
export async function readRuntimeConfig(): Promise<RuntimeConfigView> {
  return invoke<RuntimeConfigView>("read_runtime_config");
}

/**
 * Apply `patch` to `oxrsys-runtime.toml` — creates the file byte-identical to
 * the shared template first if it doesn't exist yet, otherwise snapshots a
 * backup and edits values in place, preserving every other byte. Rejects
 * when the file has a `parseError` (edits are refused rather than risking an
 * unreadable rewrite — fix the file by hand first) or on a validation/IPC
 * failure. A patch that changes nothing writes no backup and no file.
 *
 * **Rejects while a session is live**, with `./demo.sh stop --bottle <name>`
 * as the remedy. The runtime re-reads this file every 250 ms and rebuilds the
 * encoder when `encoderProcess`/`videoCodec` move, so a save mid-stream is a
 * live reconfiguration, not a next-launch setting — disable Save while
 * `blocksMutation(sessionStore.status.phase)` rather than promising "values
 * take effect at the next launch". It also serializes against
 * setup/build/install and the `edit-protocol` fix (the process-wide operation
 * lock), so a save can block briefly behind one of those.
 */
export async function writeRuntimeConfig(patch: RuntimeConfigPatch): Promise<WriteReport> {
  return invoke<WriteReport>("write_runtime_config", { patch });
}

// ── settings (Phase 4) ──────────────────────────────────────────────────────
//
// Mirrors `sabrage_core::store::settings` (via `src-tauri/src/commands.rs`).
// Persisted at `<Sabrage appsup>/settings.json`.

/** Mirrors `store::settings::LaunchDefaults` (serde camelCase) — the four
 * `run.sh`-only flags (see `LaunchOpts`'s doc comment), as app-wide defaults
 * rather than one launch's overrides. */
export interface LaunchDefaults {
  noAudio: boolean;
  noDashboard: boolean;
  wired: boolean;
  verbose: boolean;
}

/** Mirrors `store::settings::Settings` (serde camelCase).
 *
 * The index signature is the wire half of the store's `#[serde(flatten)]
 * extra` map: a newer Sabrage's keys are read and written back verbatim, so
 * running an older build and toggling one control no longer strips them.
 * Keep spreading the loaded object (`{ ...settings, ...patch }`) rather than
 * rebuilding one field by field, or the extras are dropped before they reach
 * the backend. */
export interface Settings {
  /** `store::settings::SETTINGS_VERSION` at write time. */
  version: number;
  repoRoot: string | null;
  defaultBottle: string | null;
  defaultBsDir: string | null;
  launch: LaunchDefaults;
  /** Default `true` — whether `run_doctor` is allowed to shell out to `adb`. */
  allowAdbProbes: boolean;
  /** Flips to `true` the first time the user confirms the Settings screen's
   * one-time "Sabrage edits this file in place" dialog; gates that dialog,
   * not the edit itself. */
  runtimeConfigEditAcknowledged: boolean;
  /** Top-level keys this build has no field for — a newer schema's. Opaque:
   * pass them through untouched. */
  [key: string]: unknown;
}

/** Missing `settings.json` resolves to `Settings` field defaults, not a
 * rejection — only a corrupt file rejects (never silently reset). */
export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/** Persist `settings` and return it back as saved (same shape, including any
 * unknown top-level keys, which now round-trip). Rejects when `$HOME` is
 * missing/empty/non-absolute rather than writing the store somewhere else. */
export async function saveSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("save_settings", { settings });
}

/** The Settings screen's Repository card — mirrors `commands::RepoInfo`.
 * `hostManifestLibraryPath`/`hostManifestPointsHere` read
 * `paths.host_xr_json` the way `checks::host` does; `pointsHere` is `null`
 * when the manifest itself can't be read (fresh machine, no `install` yet). */
export interface RepoInfo {
  repoRoot: string | null;
  source: "settings" | "env" | "executable" | "unresolved";
  markersPresent: boolean;
  contractSynced: boolean | null;
  hostManifestLibraryPath: string | null;
  hostManifestPointsHere: boolean | null;
  /** The `contract/` digest this Sabrage **binary** was compiled with. */
  compiledContractSha256: string;
  /** The same digest recomputed from the resolved checkout's `contract/`
   * files; `null` when there is no root to hash. */
  checkoutContractSha256: string | null;
  /** Do they agree? `false` means this build is executing a different
   * contract — different pins, ports, artifact lists and check registry —
   * than the checkout it is pointed at; rebuild Sabrage from this checkout,
   * or point `repoRoot` at the one it was built from. `contractSynced`
   * answers a different question (is `contract.gen.sh` a fresh rendering of
   * `contract/`) and can be `true` while this is `false`. */
  binaryContractMatches: boolean | null;
}

export async function getRepoInfo(): Promise<RepoInfo> {
  return invoke<RepoInfo>("get_repo_info");
}

// ── library (Phase 4) ───────────────────────────────────────────────────────
//
// Mirrors `sabrage_core::store::library` (via `src-tauri/src/commands.rs`).
// Persisted at `<Sabrage appsup>/library.json`.

/** Mirrors `store::library::LaunchOverrides` (serde camelCase) — per-game
 * overrides of `Settings.launch`; `null` on a field means "use the global
 * default", distinct from an explicit `false`. */
export interface LaunchOverrides {
  noAudio: boolean | null;
  noDashboard: boolean | null;
  wired: boolean | null;
  verbose: boolean | null;
}

/** Mirrors `store::library::LastSession` (serde camelCase) — recorded by the
 * `launch` command once a run it tagged with `gameId` settles. */
export interface LastSession {
  startedAtUnixMs: number;
  endedAtUnixMs: number;
  exitCode: number | null;
  logPath: string | null;
}

/** Mirrors `store::library::GameEntry` (serde camelCase). `id` is a v4 UUID
 * in its string form. */
export interface GameEntry {
  id: string;
  name: string;
  bsDir: string;
  bottle: string;
  appid: number;
  addedAtUnixMs: number;
  launchOverrides: LaunchOverrides;
  lastSession: LastSession | null;
}

/** Mirrors `store::library::GoldbergState` (serde camelCase) — whether
 * `steam_api64.dll` in `bsDir` is the Goldberg build Sabrage installs, the
 * untouched Steam original, something else entirely, or absent.
 *
 * `"appliedUnverified"` is Goldberg installed with **no** `.orig-steam`
 * backup: this install arrived already Goldberg'd (or the backup was
 * deleted), so no copy of the real Steam dll exists on this machine at all.
 * Deliberately not `"original"` — the bytes are provably Goldberg's — and
 * `revertOriginalSteamDll` has nothing to restore from. */
export type GoldbergState =
  | "applied"
  | "appliedUnverified"
  | "original"
  | "modified"
  | "noDll";

/** Mirrors `store::library::GameStatus` (serde camelCase) — the Library
 * table's status tag. */
export type GameStatus = "ready" | "needsAttention" | "notFound" | "needsSetup";

/** Read-only probes over one entry's `bsDir`/`bottle`, recomputed on every
 * `getLibrary`/`saveGame`/`validateGame` call (never persisted) — mirrors
 * `store::library::GameValidity`. `problems` is one human line per failing
 * rule, in the order the UI's red lines should render them. */
export interface GameValidity {
  exePresent: boolean;
  detectedVersion: string | null;
  versionOk: boolean;
  bottleExists: boolean;
  bottleTemplate: string | null;
  bottleBackendDxmt: boolean;
  outsideDriveC: boolean;
  /** `null` unless `outsideDriveC` — whether the bottle's `z:` drive link
   * exists (a missing one is why "outside drive_c" games fail to launch). */
  zDriveOk: boolean | null;
  goldberg: GoldbergState;
  origSteamPresent: boolean;
  status: GameStatus;
  problems: string[];
}

/** One `getLibrary` row: a saved entry plus its freshly computed validity —
 * mirrors `commands::GameRow` (there is no separate persisted "row" shape;
 * this is assembled per call). */
export interface GameRow {
  entry: GameEntry;
  validity: GameValidity;
}

export async function getLibrary(): Promise<GameRow[]> {
  return invoke<GameRow[]>("get_library");
}

/** A fresh, not-yet-saved `GameEntry` prefilled from `Settings` and the
 * detected bottles — the Library screen's "Add game" starting point. Save it
 * (unchanged or edited) with `saveGame` to add it to the library. */
export async function newGameTemplate(): Promise<GameEntry> {
  return invoke<GameEntry>("new_game_template");
}

/** Upsert `entry` into the library by `id` and return its row (the entry as
 * saved, plus freshly computed validity). */
export async function saveGame(entry: GameEntry): Promise<GameRow> {
  return invoke<GameRow>("save_game", { entry });
}

/** Remove the entry named `id`. Resolves `false` when no such entry exists
 * (already removed, stale UI state) rather than rejecting. */
export async function removeGame(id: string): Promise<boolean> {
  return invoke<boolean>("remove_game", { id });
}

/** Run the same read-only probes `getLibrary`/`saveGame` use, against an
 * arbitrary `bsDir`/`bottle` pair that need not belong to a saved entry yet —
 * the EditGame form's live (debounced) validation. */
export async function validateGame(bsDir: string, bottle: string): Promise<GameValidity> {
  return invoke<GameValidity>("validate_game", { bsDir, bottle });
}

/** Mirrors `store::goldberg::RevertReport`. */
export interface RevertReport {
  restored: boolean;
  message: string;
  dllPath: string;
}

/**
 * Restore `steam_api64.dll.orig-steam` back over `steam_api64.dll` for the
 * library entry named `gameId`. A no-op (`restored: false`, explanatory
 * `message`) when there is no `.orig-steam` to restore from, or when the
 * backup is itself the pinned Goldberg dll — nothing here can prove a backup
 * is the real Steam dll, so the message says "the .orig-steam backup", never
 * "the original". The next launch re-applies Goldberg regardless — `message`
 * says so (`// DIVERGENCE:` from run.sh, which never restores; see
 * `PARITY.md`). Rejects while a session is live.
 *
 * `expectedBsDir` is the Beat Saber directory the screen showed and
 * validated. This command mutates the entry's **saved** `bsDir`, so a form
 * with an unsaved path edit would otherwise overwrite a dll in a different
 * installation than the one on screen; pass it and the backend fails closed
 * on a mismatch. Optional only so a caller that renders no path can omit it.
 */
export async function revertOriginalSteamDll(
  gameId: string,
  expectedBsDir?: string | null,
): Promise<RevertReport> {
  return invoke<RevertReport>("revert_original_steam_dll", {
    gameId,
    expectedBsDir: expectedBsDir ?? null,
  });
}

// ── native dialogs ───────────────────────────────────────────────────────────

/**
 * Open a native "choose a folder" dialog (`@tauri-apps/plugin-dialog`,
 * `dialog:allow-open` capability). Resolves `null` on cancel — the plugin's
 * own `open()` returns `string | string[] | null` depending on options; this
 * always asks for a single directory, so the union collapses to
 * `string | null` here.
 */
/** `commands::BsDirSuggestion` — the bottle-derived Beat Saber dir (empty when
 * no bottle) and the nearest EXISTING directory a folder picker should start
 * in (the field's own value, else the derived path, else $HOME). */
export interface BsDirSuggestion {
  derived: string;
  browseStart: string;
}

/** `suggest_bs_dir` — read-only; feeds Browse… and the empty-field placeholder. */
export async function suggestBsDir(
  bottle: string | null,
  current: string | null,
): Promise<BsDirSuggestion> {
  return invoke<BsDirSuggestion>("suggest_bs_dir", { bottle, current });
}

export async function pickFolder(
  title: string,
  defaultPath?: string | null,
): Promise<string | null> {
  const selected = await open({
    directory: true,
    multiple: false,
    title,
    defaultPath: defaultPath ?? undefined,
  });
  return typeof selected === "string" ? selected : null;
}
