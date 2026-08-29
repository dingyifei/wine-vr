// Hand-mirrored IPC boundary between the Svelte frontend and sabrage-app's
// Tauri commands (`src-tauri/src/commands.rs`). No codegen — when either side
// changes a shape, update both by hand and keep this comment as the pointer.

import { Channel, invoke } from "@tauri-apps/api/core";

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

/** Mirrors `sabrage_core::Stage` (serde lowercase). `"run"` exists in the type
 * but `run_stage` always resolves it to a `fatal` event — the run stage lands
 * in Phase 3. */
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
  | { kind: "stageFinished"; runId: string; stage: Stage; ok: boolean; exitCodeEquiv: number };

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

/** Run the `stop` stage for `bottle`. */
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
