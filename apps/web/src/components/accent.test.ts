import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

function createStorage(initial: Record<string, string> = {}) {
  const store = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
    removeItem: (key: string) => void store.delete(key),
    clear: () => store.clear(),
    key: (index: number) => [...store.keys()][index] ?? null,
    get length() {
      return store.size;
    },
  };
}

function createDocument() {
  const props: Record<string, string> = {};
  return {
    documentElement: {
      dataset: {} as Record<string, string>,
      style: {
        props,
        setProperty: (name: string, value: string) => void (props[name] = value),
        getPropertyValue: (name: string) => props[name] ?? "",
      },
      lang: "",
    },
  };
}

describe("accent color", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.stubGlobal("localStorage", createStorage());
    vi.stubGlobal("document", createDocument());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uses green by default and applies it per theme", async () => {
    const accent = await import("./accent");
    expect(accent.getAccent()).toBe("green");

    accent.applyAccent("light");
    expect(document.documentElement.style.getPropertyValue("--accent")).toBe("#167c5b");
    expect(document.documentElement.style.getPropertyValue("--accent-soft")).toBe("#e4f3ed");

    accent.applyAccent("dark");
    expect(document.documentElement.style.getPropertyValue("--accent")).toBe("#54c79a");
    expect(document.documentElement.style.getPropertyValue("--accent-soft")).toBe("#153c2d");
  });

  it("persists a selected accent and applies it immediately", async () => {
    document.documentElement.dataset.theme = "dark";
    const accent = await import("./accent");
    accent.setAccent("blue");

    expect(accent.getAccent()).toBe("blue");
    expect(localStorage.getItem("mix-agent-accent")).toBe("blue");
    expect(document.documentElement.style.getPropertyValue("--accent")).toBe("#6ba3ff");
  });

  it("restores a saved accent and ignores unknown values", async () => {
    localStorage.setItem("mix-agent-accent", "violet");
    let accent = await import("./accent");
    expect(accent.getAccent()).toBe("violet");

    vi.resetModules();
    localStorage.setItem("mix-agent-accent", "not-a-color");
    accent = await import("./accent");
    expect(accent.getAccent()).toBe("green");
  });
});
