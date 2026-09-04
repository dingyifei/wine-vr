// `./demo.sh …` command lines for display. `demoRunCommand` builds the `run`
// equivalent of a `LaunchOpts` — the "equivalent demo.sh command" line under the
// Session screen's launch controls and in the Settings footer (design-app.md §4);
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
 * Flag order is `--bottle` → `--bs-dir` → `--no-audio` → `--no-dashboard` →
 * `--wired` → `--verbose`; every flag but `--bottle`/`--bs-dir` is a bare switch
 * with no `=value` form, matching `demo.sh`'s own parser. `bottle`/`bsDir` go
 * through `shQuote`. `opts.bottle` renders as the literal `<name>` when falsy,
 * untrimmed; `opts.bsDir` is trimmed and the whole `--bs-dir …` pair is omitted
 * when it trims to empty. `gameId`/`dryRun` have no `demo.sh run` flag and are ignored.
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
