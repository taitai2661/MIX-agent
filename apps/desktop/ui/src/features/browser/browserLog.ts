/** One row in the browser pane's activity list. */
export interface LogEntry {
  id: number;
  action: string;
  ok: boolean;
  summary: string;
  detail?: string;
  artifactId?: string | null;
}

let nextId = 0;

/** Monotonic ids keep React keys stable across the two producers (form and
 * install actions) without a `crypto.randomUUID()` per row. */
export function makeLogEntry(entry: Omit<LogEntry, "id">): LogEntry {
  return { id: nextId++, ...entry };
}
