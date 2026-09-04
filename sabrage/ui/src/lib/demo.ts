// `./demo.sh …` command lines for display. `demoRunCommand` builds the `run`
// equivalent of a `LaunchOpts` — the "equivalent demo.sh command" footer on the
// Session and Settings screens (design-app.md §4); both call it, so identical
// options yield byte-identical text. `shQuote` is also used by StagesPanel.

import type { LaunchOpts } from "../ipc";

/**
 * Quote `v` for use in a copy-pasted zsh command line. Single-quotes the value
 * with embedded apostrophes escaped, so no shell metacharacter is interpreted;
 * values already safe bare and the literal `<name>` placeholder are returned
 * unquoted.
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
