// The `./demo.sh run …` command line equivalent to a given `LaunchOpts` — the
// "equivalent demo.sh command" line shown under the Session and Library
// screens' launch controls (design-app.md §4). Moved out of Session.svelte in
// Phase 4 so both screens render byte-identical output for the same options;
// port `demoRunCommand` changes back into Session.svelte's copy-button call
// site rather than re-deriving the string there.

import type { LaunchOpts } from "../ipc";

/**
 * Single-quote a value for safe use in a copy-pasted zsh command line —
 * single-quote encoding, embedded apostrophes escaped as `'\''` (close the
 * quote, an escaped literal apostrophe, reopen the quote). Unlike bare or
 * double-quoted interpolation, single quotes disable every shell
 * metacharacter inside them — `$()`/backticks/`$VAR`/`"` all come through
 * literally, so a pasted path can't expand or execute anything.
 *
 * A value already safe bare (matches the allow-list below) is returned
 * unquoted so the common case still reads like the README's plain examples;
 * the literal `<name>` placeholder is always returned bare regardless — it
 * is not a real value the shell will ever see, quoting it would just make
 * the template read oddly (`'<name>'`).
 */
export function shQuote(v: string): string {
  if (v === "<name>") return v;
  if (/^[A-Za-z0-9_.\/:@%+=-]+$/.test(v)) return v;
  return `'${v.replace(/'/g, `'\\''`)}'`;
}

/**
 * Mirrors Session.svelte's original `equivalentCommand()` — same flag order
 * (`--bottle` → `--bs-dir` → `--no-audio` → `--no-dashboard` → `--wired` →
 * `--verbose`); every other flag is a bare switch, no `=value` form, matching
 * `demo.sh`'s own parser. `bottle`/`bsDir` are now quoted with `shQuote`
 * (single-quote encoding) rather than bare double quotes — a bottle or
 * directory containing `$()`, backticks, `"`, `\`, or `'` used to come
 * through unescaped, so pasting the copied command could execute the
 * embedded shell syntax instead of passing it through as one argument.
 *
 * `opts.bottle` renders as the literal placeholder `<name>` when falsy (no
 * bottle chosen yet) — not trimmed first, matching the original's
 * `selectedBottle || "<name>"`. `opts.bsDir` is trimmed and the whole
 * `--bs-dir …` pair is omitted when that trims to empty. `gameId`/`dryRun`
 * have no `demo.sh run` flag and are ignored.
 */
export function demoRunCommand(opts: LaunchOpts): string {
  const bottle = opts.bottle || "<name>";
  const parts = ["./demo.sh", "run", "--bottle", shQuote(bottle)];
  const bsDir = (opts.bsDir ?? "").trim();
  if (bsDir) parts.push("--bs-dir", shQuote(bsDir));
  if (opts.noAudio) parts.push("--no-audio");
  if (opts.noDashboard) parts.push("--no-dashboard");
  if (opts.wired) parts.push("--wired");
  if (opts.verbose) parts.push("--verbose");
  return parts.join(" ");
}
