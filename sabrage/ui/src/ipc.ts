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
