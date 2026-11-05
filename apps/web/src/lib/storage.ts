/**
 * localStorage access that cannot crash the app.
 *
 * Private browsing, embedded WebViews and enterprise policies can make the
 * `localStorage` getter itself throw. Several modules read it during render or
 * module evaluation, so one exception used to blank the whole UI before the
 * error boundary could mount.
 */
function store(): Storage | null {
  try {
    if (typeof localStorage === "undefined") return null;
    return localStorage;
  } catch {
    return null;
  }
}

export const safeStorage = {
  get(key: string): string | null {
    try {
      return store()?.getItem(key) ?? null;
    } catch {
      return null;
    }
  },
  set(key: string, value: string): void {
    try {
      store()?.setItem(key, value);
    } catch {
      // Quota exceeded / disabled storage: keep the in-memory state working.
    }
  },
  remove(key: string): void {
    try {
      store()?.removeItem(key);
    } catch {
      // Ignore.
    }
  },
};
