/// Centralised WebStorage access.
///
/// Every `localStorage` call used to be inline with its own try/catch, which
/// spread the "storage is best-effort" knowledge over six files and made the
/// key space ungreppable. All persistent UI state goes through these helpers:
///
/// - every operation is best-effort (private windows / disabled storage throw)
/// - JSON values are validated at the boundary, never in callers
/// - the backing store is injectable so tests pass a memory stub instead of
///   touching the global
///
/// Durable prefs stay in `localStorage` (see `package.json`: no store plugin —
/// the Tauri `LazyStore` would buy crash-safe writes the UI does not need, at
/// the cost of an async API every reader would have to await).

function resolveStore(store?: Storage | null): Storage | null {
  if (store !== undefined) return store;
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** Read a raw string. Returns `fallback` when missing or unreadable. */
export function getString(
  key: string,
  fallback: string | null = null,
  store?: Storage | null,
): string | null {
  const s = resolveStore(store);
  if (!s) return fallback;
  try {
    return s.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

/** Write a raw string. Never throws. */
export function setString(
  key: string,
  value: string,
  store?: Storage | null,
): void {
  const s = resolveStore(store);
  if (!s) return;
  try {
    s.setItem(key, value);
  } catch {
    // Persisting UI state is best-effort; the in-memory state still applies.
  }
}

/** Remove a key. Never throws. */
export function removeKey(key: string, store?: Storage | null): void {
  const s = resolveStore(store);
  if (!s) return;
  try {
    s.removeItem(key);
  } catch {
    // Best-effort (see `setString`).
  }
}

/** Every stored key, optionally narrowed to a prefix. Best-effort. */
export function listKeys(prefix = "", store?: Storage | null): string[] {
  const s = resolveStore(store);
  if (!s) return [];
  try {
    const keys: string[] = [];
    for (let i = 0; i < s.length; i += 1) {
      const key = s.key(i);
      if (key !== null && key.startsWith(prefix)) keys.push(key);
    }
    return keys;
  } catch {
    return [];
  }
}

/** Remove every key under a prefix. Returns the number removed. */
export function removeKeys(prefix: string, store?: Storage | null): number {
  const keys = listKeys(prefix, store);
  for (const key of keys) removeKey(key, store);
  return keys.length;
}

/** Read a `"1"` / `"0"` boolean. Returns `fallback` when missing/unreadable. */
export function getBoolean(
  key: string,
  fallback: boolean,
  store?: Storage | null,
): boolean {
  const raw = getString(key, null, store);
  if (raw === null) return fallback;
  return raw === "1";
}

/** Write a `"1"` / `"0"` boolean. Never throws. */
export function setBoolean(
  key: string,
  value: boolean,
  store?: Storage | null,
): void {
  setString(key, value ? "1" : "0", store);
}

/** Read a JSON value guarded by `validate`. Returns `fallback` on any failure. */
export function getJSON<T>(
  key: string,
  fallback: T,
  validate: (value: unknown) => value is T,
  store?: Storage | null,
): T {
  const raw = getString(key, null, store);
  if (!raw) return fallback;
  try {
    const parsed: unknown = JSON.parse(raw);
    return validate(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

/** Write a JSON value. Never throws. */
export function setJSON(
  key: string,
  value: unknown,
  store?: Storage | null,
): void {
  try {
    setString(key, JSON.stringify(value), store);
  } catch {
    // `JSON.stringify` on cyclic input throws synchronously — still best-effort.
  }
}

/** In-memory `Storage` stub for tests. Mirrors the WebStorage API. */
export function memoryStore(entries: Record<string, string> = {}): Storage {
  const map = new Map(Object.entries(entries));
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (key: string) => map.get(key) ?? null,
    key: (index: number) => [...map.keys()][index] ?? null,
    removeItem: (key: string) => {
      map.delete(key);
    },
    setItem: (key: string, value: string) => {
      map.set(key, value);
    },
  };
}
