// Hand-mirrored IPC boundary between the Svelte frontend and sabrage-app's
// Tauri commands (`src-tauri/src/commands.rs`). No codegen — when either side
// changes a shape, update both by hand and keep this comment as the pointer.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
   * `contractFixIdToAction` before offering a Fix button — two contract ids
   * (`fix.create-z-drive`, `fix.edit-protocol`) are deliberately unmodelled
   * and resolve to `null`. */
  fix: string | null;
}

/** The aggregate `run_doctor` resolves to — mirrors `commands::DoctorSummary`. */
export interface DoctorSummary {
  failCount: number;
  warnCount: number;
  total: number;
}

/** Sidebar footer snapshot — mirrors `commands::AppState`. */
export interface AppState {
  repoRoot: string | null;
  bottles: string[];
  alvrVersion: string;
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

/** Mirrors `sabrage_core::FixAction` (serde kebab-case — the contract id
 * without its `fix.` prefix). */
export type FixAction =
  | "set-graphics-backend"
  | "restage-helper"
  | "remove-adb-forwards"
  | "delete-session-json"
  | "run-setup"
  | "run-build"
  | "run-install";

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
  | { kind: "output"; runId: string; step: string; stream: StreamKind; chunk: string }
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
 * `deleteSessionJson` is destructive; `stage` names the whole-stage fixes
 * whose intended UI path is `runStage` directly (see `contractFixIdToAction`'s
 * doc comment) rather than `applyFix`. */
export const FIX_META: Record<
  FixAction,
  { title: string; needsAdmin: boolean; destructive: boolean; stage: Stage | null }
> = {
  "set-graphics-backend": {
    title: "force the bottle's graphics backend to dxmt",
    needsAdmin: false,
    destructive: false,
    stage: null,
  },
  "restage-helper": {
    title: "restage the arm64 encoder helper",
    needsAdmin: false,
    destructive: false,
    stage: null,
  },
  "remove-adb-forwards": {
    title: "remove the stale adb port forwards",
    needsAdmin: false,
    destructive: false,
    stage: null,
  },
  "delete-session-json": {
    title: "delete ALVR's session.json (clears pinned client IPs)",
    needsAdmin: false,
    destructive: true,
    stage: null,
  },
  "run-setup": { title: "run setup", needsAdmin: false, destructive: false, stage: "setup" },
  "run-build": { title: "run build", needsAdmin: false, destructive: false, stage: "build" },
  "run-install": { title: "run install", needsAdmin: true, destructive: false, stage: "install" },
};

/** `"fix.set-graphics-backend"` -> `"set-graphics-backend"`; `null` for a
 * contract id this table does not model — mirrors
 * `FixAction::from_contract_id`'s two deliberately-deferred ids
 * (`fix.create-z-drive`, `fix.edit-protocol`). A `null` result means "render
 * no Fix button", not an error. */
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
 * word is already lowercase, so this looks like plain lowercase). */
export type SessionPhase =
  | "idle"
  | "preflight"
  | "launching"
  | "running"
  | "stalled"
  | "stopping"
  | "exited"
  | "detached";

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
 * itself — see that struct's own doc comment. */
export interface LaunchOpts {
  bottle?: string | null;
  bsDir?: string | null;
  noAudio?: boolean | null;
  noDashboard?: boolean | null;
  wired?: boolean | null;
  verbose?: boolean | null;
  dryRun?: boolean | null;
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
  | { kind: "dead"; state: SessionState; restored: string[] }
  | { kind: "identityMismatch"; state: SessionState; restored: string[] };

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

/** Stop a tail started by `startLogTail`. */
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
 * `"cancel"` reliably returns control to the caller. */
export async function resolveQuit(choice: QuitChoice): Promise<void> {
  return invoke("resolve_quit", { choice });
}

// ── global events ────────────────────────────────────────────────────────────

/** Subscribe to the 1 Hz `session://status` broadcast. */
export async function onSessionStatus(cb: (status: SessionStatus) => void): Promise<UnlistenFn> {
  return listen<SessionStatus>("session://status", (event) => cb(event.payload));
}

/** Subscribe to `app://quit-requested` — fired when `ExitRequested`/
 * `CloseRequested` was intercepted because a session is still live. */
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
