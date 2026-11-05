/** Lightweight performance marks for the start-up path.
 *
 * The marks show up in the WebView devtools timeline and can be read back with
 * `performance.getEntriesByType("mark")`. They are intentionally cheap and
 * always on: a `performance.mark` is a timestamp write, not a timer.
 */
export function mark(name: string): void {
  if (
    typeof performance !== "undefined" &&
    typeof performance.mark === "function"
  ) {
    performance.mark(name);
  }
}

const seen = new Set<string>();

/** Mark only the first occurrence of a lifecycle name. */
export function markOnce(name: string): void {
  if (seen.has(name)) return;
  seen.add(name);
  mark(name);
}
