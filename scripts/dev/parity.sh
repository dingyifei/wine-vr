#!/bin/zsh
# scripts/dev/parity.sh — the Sabrage (Rust) <-> demo.sh (zsh) parity harness.
# See sabrage/docs/design/design-parity.md §4 for the design this implements.
#
# Usage:
#   scripts/dev/parity.sh [--bottle <name>] [--live=off]
#       Tier 1: cargo test -p sabrage-parity -p sabrage-contract-gen (always).
#       Tier 2: live doctor diff (zsh doctor vs `sabrage doctor`) — runs only when
#       a bottle is known (--bottle, else $WINEVR_BOTTLE) and --live=off is absent.
#
#   scripts/dev/parity.sh --regen
#       cargo run -p sabrage-contract-gen -- --write, then re-run tier 1, then bless
#       the shell fingerprint (see --bless). Use after editing contract/pipeline.toml.
#
#   scripts/dev/parity.sh --bless
#       Recompute sabrage/parity/shell.fingerprint and write it, but ONLY if tier 1
#       (with the fingerprint assertion itself skipped — see PARITY_SKIP_FINGERPRINT
#       below) passes. Run this after an intentional edit to demo.sh/scripts/demo/*.sh.
#
#   scripts/dev/parity.sh --install-hook
#       git config core.hooksPath scripts/dev/hooks — NOTE: this setting lives in
#       .git/config and applies to EVERY worktree that shares this git directory,
#       not just the one you ran it from.
#
# Env:
#   WINEVR_BOTTLE               same var demo.sh reads; used as the default --bottle
#                                for the tier-2 live differ when --bottle is absent.
#   PARITY_SKIP_FINGERPRINT=1   Contract with the sabrage-parity tier-1 test: when
#                                set, its shell-fingerprint assertion must skip itself
#                                (not fail, not fabricate a pass — just not run) so
#                                `--bless` can validate everything else before the
#                                fingerprint file it is about to overwrite is checked
#                                against itself.
#
# This script never mutates anything under scripts/demo/, contract/, or demo.sh
# itself. It writes only: sabrage/parity/shell.fingerprint (--bless/--regen),
# scripts/demo/contract.gen.sh via the sabrage-contract-gen binary (--regen only,
# and only that binary touches the file), and .git/config (--install-hook only).

set -u

SELF="${0:A}"
ROOT="$(cd "$(dirname "$SELF")/../.." && pwd)"
SABRAGE="$ROOT/sabrage"
FPRINT="$SABRAGE/parity/shell.fingerprint"
CONTRACT_TOML="$ROOT/contract/pipeline.toml"

export PATH="$HOME/.cargo/bin:$PATH"

_G=$'\e[32m'; _Y=$'\e[33m'; _R=$'\e[31m'; _N=$'\e[0m'
say()  { print -r -- "$*" }
ok()   { print -r -- "  ${_G}OK${_N}   $*" }
warn() { print -r -- "  ${_Y}WARN${_N} $*" }
fail() { print -r -- "  ${_R}FAIL${_N} $*" }

usage() {
  sed -n '2,33p' "$SELF" | sed 's/^# \{0,1\}//'
}

require_cargo() {
  command -v cargo >/dev/null 2>&1 || {
    print -r -- "parity.sh: cargo not found on PATH (tried \$HOME/.cargo/bin too)" >&2
    exit 1
  }
}

# ---- tier 1 -------------------------------------------------------------------

run_tier1() { # [skip_fingerprint: 0|1]
  local skip="${1:-0}"
  if [ "$skip" = 1 ]; then
    say "== tier 1: cargo test -p sabrage-parity -p sabrage-contract-gen (PARITY_SKIP_FINGERPRINT=1) =="
    ( cd "$SABRAGE" && PARITY_SKIP_FINGERPRINT=1 cargo test -p sabrage-parity -p sabrage-contract-gen )
  else
    say "== tier 1: cargo test -p sabrage-parity -p sabrage-contract-gen =="
    ( cd "$SABRAGE" && cargo test -p sabrage-parity -p sabrage-contract-gen )
  fi
}

# ---- shell fingerprint ---------------------------------------------------------
# sabrage/parity/shell.fingerprint format (the contract the sabrage-parity tier-1
# fingerprint test must match): one line per file,
#   "<sha256 hex>  <path-relative-to-repo-root>\n"
# (two spaces, matching `shasum -a 256` output), sorted ascending by path (byte/
# ASCII sort), no header, no comments, LF line endings, trailing newline on the
# last line. Covers demo.sh + every scripts/demo/*.sh EXCEPT the generated
# scripts/demo/contract.gen.sh — that file's byte parity is already covered by the
# sabrage-contract-gen --check golden test, and fingerprinting it too would dirty
# this file on every --regen even when no hand-written shell line changed.

fingerprint_relpaths() {
  local f
  print -r -- "demo.sh"
  for f in "$ROOT"/scripts/demo/*.sh; do
    [ "$(basename "$f")" = "contract.gen.sh" ] && continue
    print -r -- "scripts/demo/$(basename "$f")"
  done
}

compute_fingerprint() {
  local rel
  fingerprint_relpaths | sort | while IFS= read -r rel; do
    shasum -a 256 "$ROOT/$rel" | awk -v p="$rel" '{print $1"  "p}'
  done
}

do_bless() {
  require_cargo
  say "== bless: recompute + write $FPRINT =="
  if ! run_tier1 1; then
    fail "tier 1 (fingerprint assertion skipped) did not pass — fix that first, don't bless around it"
    return 1
  fi
  mkdir -p "$(dirname "$FPRINT")"
  compute_fingerprint > "$FPRINT"
  ok "wrote $FPRINT"
  return 0
}

# ---- regen ----------------------------------------------------------------------

do_regen() {
  require_cargo
  say "== regen: cargo run -p sabrage-contract-gen -- --write =="
  if ! ( cd "$SABRAGE" && cargo run -q -p sabrage-contract-gen -- --write ); then
    fail "sabrage-contract-gen --write failed"
    return 1
  fi
  if ! run_tier1 0; then
    fail "tier 1 failed after regen — not blessing"
    return 1
  fi
  do_bless
}

# ---- install-hook -----------------------------------------------------------------

do_install_hook() {
  git -C "$ROOT" config core.hooksPath scripts/dev/hooks || { fail "git config failed"; return 1; }
  ok "core.hooksPath = scripts/dev/hooks"
  warn "this is stored in .git/config and affects EVERY worktree sharing this git directory, not just this checkout"
  return 0
}

# ---- tier 2: live doctor diff -------------------------------------------------

volatile_slugs() { # slugs the contract marks volatile = true (compare presence only)
  awk '
    BEGIN { RS=""; FS="\n" }
    {
      slug = ""; volatile = 0
      for (i = 1; i <= NF; i++) {
        line = $i
        if (line ~ /^slug = /)        { slug = line; gsub(/^slug = "/, "", slug); gsub(/"$/, "", slug) }
        if (line ~ /^volatile = true/) { volatile = 1 }
      }
      if (volatile == 1 && slug != "") print slug
    }
  ' "$CONTRACT_TOML"
}

diff_tap() { # t1(zsh) t2(native) -> prints a mismatch table, returns 0/1
  local t1="$1" t2="$2"
  local -A zsh_status native_status volatile seen
  local slug st line

  while IFS=' ' read -r slug st; do
    [ -n "$slug" ] && zsh_status[$slug]="$st"
  done < "$t1"

  while IFS=' ' read -r slug st; do
    [ -n "$slug" ] && native_status[$slug]="$st"
  done < "$t2"

  local vslugs v
  vslugs="$(volatile_slugs)"
  while IFS= read -r v; do
    [ -n "$v" ] && volatile[$v]=1
  done <<< "$vslugs"

  local all_slugs=()
  for slug in "${(@k)zsh_status}" "${(@k)native_status}"; do
    [ -n "${seen[$slug]:-}" ] && continue
    seen[$slug]=1
    all_slugs+=("$slug")
  done
  all_slugs=("${(@o)all_slugs}")

  local mismatches=0
  printf '  %-28s %-10s %-10s %s\n' "SLUG" "ZSH" "NATIVE" "NOTE"
  for slug in "${all_slugs[@]}"; do
    local zv="${zsh_status[$slug]:-}" nv="${native_status[$slug]:-}"
    if [ -z "$zv" ]; then
      printf '  %-28s %-10s %-10s %s\n' "$slug" "(absent)" "$nv" "missing on zsh"
      mismatches=$((mismatches + 1))
    elif [ -z "$nv" ]; then
      printf '  %-28s %-10s %-10s %s\n' "$slug" "$zv" "(absent)" "missing on native"
      mismatches=$((mismatches + 1))
    elif [ -n "${volatile[$slug]:-}" ]; then
      : # both sides ran this slug; volatile checks compare presence only
    elif [ "$zv" != "$nv" ]; then
      printf '  %-28s %-10s %-10s %s\n' "$slug" "$zv" "$nv" "status mismatch"
      mismatches=$((mismatches + 1))
    fi
  done

  local zsh_fails=0 native_fails=0
  for slug in "${(@k)zsh_status}"; do [ "${zsh_status[$slug]}" = "fail" ] && zsh_fails=$((zsh_fails + 1)); done
  for slug in "${(@k)native_status}"; do [ "${native_status[$slug]}" = "fail" ] && native_fails=$((native_fails + 1)); done
  if [ "$zsh_fails" != "$native_fails" ]; then
    printf '  %-28s %-10s %-10s %s\n' "FAILCOUNT" "$zsh_fails" "$native_fails" "fail-count mismatch"
    mismatches=$((mismatches + 1))
  fi

  if [ "$mismatches" = 0 ]; then
    ok "tier 2: ${#all_slugs[@]} slugs agree (fail-count $zsh_fails == $native_fails)"
    return 0
  fi
  fail "tier 2: $mismatches mismatch(es)"
  return 1
}

do_tier2() { # bottle
  local bottle="$1"
  say "== tier 2: live doctor diff (bottle=$bottle) =="

  local scratch
  scratch="$(mktemp -d "${TMPDIR:-/tmp}/wine-vr-parity.XXXXXX")" || { fail "mktemp failed"; return 1; }
  local t1="$scratch/zsh.tap" t2="$scratch/native.tap"
  : > "$t1"
  : > "$t2"

  say "-- WINEVR_DOCTOR_TAP=$t1 WINEVR_DOCTOR_SOFT=1 ./demo.sh doctor --bottle $bottle"
  ( WINEVR_DOCTOR_TAP="$t1" WINEVR_DOCTOR_SOFT=1 "$ROOT/demo.sh" doctor --bottle "$bottle" ) >/dev/null
  [ -s "$t1" ] || warn "zsh doctor produced no tap output ($t1) — doctor may have died before section 3"

  local bin="$SABRAGE/target/debug/sabrage"
  # Always rebuild: diffing a stale binary reports parity for code that is not
  # running — exactly the failure mode this differ exists to catch. A no-op
  # when nothing changed.
  say "-- cargo build -q -p sabrage-cli"
  if ! ( cd "$SABRAGE" && cargo build -q -p sabrage-cli ); then
    fail "cargo build -p sabrage-cli failed"
    rm -rf "$scratch"
    return 1
  fi

  say "-- $bin doctor --bottle $bottle --tap $t2"
  ( "$bin" doctor --bottle "$bottle" --tap "$t2" ) >/dev/null 2>&1

  diff_tap "$t1" "$t2"
  local rc=$?
  rm -rf "$scratch"
  return $rc
}

# ---- default: tier 1 always, tier 2 if a bottle is known ----------------------

run_default() {
  local bottle="${WINEVR_BOTTLE:-}"
  local live=1

  while [ $# -gt 0 ]; do
    case "$1" in
      --bottle)
        [ $# -ge 2 ] || { print -r -- "parity.sh: --bottle needs a name" >&2; exit 2; }
        bottle="$2"; shift 2 ;;
      --live=off) live=0; shift ;;
      --live=on)  live=1; shift ;;
      *) print -r -- "parity.sh: unknown argument '$1'" >&2; exit 2 ;;
    esac
  done

  require_cargo
  run_tier1 0
  local tier1_rc=$?

  local tier2_rc=0
  if [ "$live" = 1 ] && [ -n "$bottle" ]; then
    do_tier2 "$bottle"
    tier2_rc=$?
  elif [ "$live" = 1 ]; then
    say "== tier 2: skipped (no bottle — pass --bottle <name> or set WINEVR_BOTTLE) =="
  else
    say "== tier 2: skipped (--live=off) =="
  fi

  [ "$tier1_rc" = 0 ] && [ "$tier2_rc" = 0 ]
}

# ---- dispatch -------------------------------------------------------------------

case "${1:-}" in
  --regen)        do_regen;        exit $? ;;
  --bless)        do_bless;        exit $? ;;
  --install-hook) do_install_hook; exit $? ;;
  -h|--help)      usage;           exit 0 ;;
  *)              run_default "$@"; exit $? ;;
esac
