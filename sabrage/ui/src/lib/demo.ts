// The `./demo.sh run …` command line equivalent to a given `LaunchOpts` — the
// "equivalent demo.sh command" line shown under the Session and Library
// screens' launch controls (design-app.md §4). Moved out of Session.svelte in
// Phase 4 so both screens render byte-identical output for the same options;
// port `demoRunCommand` changes back into Session.svelte's copy-button call
// site rather than re-deriving the string there.

import type { LaunchOpts } from "../ipc";

/**
 * Mirrors Session.svelte's original `equivalentCommand()` exactly — same flag
 * order (`--bottle` → `--bs-dir` → `--no-audio` → `--no-dashboard` →
 * `--wired` → `--verbose`) and same quoting (`bsDir` double-quoted verbatim,
 * no escaping; every other flag is a bare switch, no `=value` form, matching
 * `demo.sh`'s own parser).
 *
 * `opts.bottle` renders as the literal placeholder `<name>` when falsy (no
 * bottle chosen yet) — not trimmed first, matching the original's
 * `selectedBottle || "<name>"`. `opts.bsDir` is trimmed and the whole
 * `--bs-dir "…"` pair is omitted when that trims to empty. `gameId`/`dryRun`
 * have no `demo.sh run` flag and are ignored.
 */
export function demoRunCommand(opts: LaunchOpts): string {
  const bottle = opts.bottle || "<name>";
  // Quote exactly like `--bs-dir` below whenever the shell would otherwise
  // split it (a bottle named "Beat Saber" is legal in CrossOver) — a bare
  // name stays bare so the common case reads like the README's examples.
  const parts = ["./demo.sh", "run", "--bottle", /[^A-Za-z0-9_.<>-]/.test(bottle) ? `"${bottle}"` : bottle];
  const bsDir = (opts.bsDir ?? "").trim();
  if (bsDir) parts.push("--bs-dir", `"${bsDir}"`);
  if (opts.noAudio) parts.push("--no-audio");
  if (opts.noDashboard) parts.push("--no-dashboard");
  if (opts.wired) parts.push("--wired");
  if (opts.verbose) parts.push("--verbose");
  return parts.join(" ");
}
