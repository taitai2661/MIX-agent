import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Provide an in-memory `localStorage` stand-in for tests that run in the
// `node` environment (e.g. the ui store tests that don't import DOM APIs).
// Vitest decides the environment per-file, so pure-store tests may not get
// jsdom's built-in `localStorage`. The fallback mirrors the WebStorage API
// surface that `useUi` actually uses (getItem/setItem/removeItem/clear).
if (typeof globalThis.localStorage === "undefined") {
  const store = new Map<string, string>();
  const fake: Storage = {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: (key) => (store.has(key) ? store.get(key)! : null),
    key: (index) => Array.from(store.keys())[index] ?? null,
    removeItem: (key) => {
      store.delete(key);
    },
    setItem: (key, value) => {
      store.set(key, String(value));
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    value: fake,
    writable: true,
    configurable: true,
  });
}

afterEach(() => {
  cleanup();
});
