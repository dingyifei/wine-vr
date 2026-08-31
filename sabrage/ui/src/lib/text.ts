// Small text-formatting helpers shared across screens/components. Pulled out
// of Doctor.svelte/GateModal.svelte (which had byte-identical copies) and the
// ~25 `catch (e)` sites that all hand-rolled the same
// `e instanceof Error ? e.message : String(e)` one-liner.

/** Capitalize the first character; empty string passes through unchanged. */
export function cap(s: string): string {
  return s.length ? s[0].toUpperCase() + s.slice(1) : s;
}

/** `"some-group"` -> `"Some Group"` — Doctor's check-group headers. */
export function titleCase(group: string): string {
  return group
    .split("-")
    .map((w) => (w.length ? w[0].toUpperCase() + w.slice(1) : w))
    .join(" ");
}

/** The message text for a caught value that may or may not be an `Error` —
 * every `catch (e)` block in this UI wants exactly this. */
export function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
