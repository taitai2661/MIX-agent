import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  ACCENTS,
  ACCENT_STORAGE_KEY,
  DEFAULT_ACCENT,
  applyAccent,
  normalizeAccent,
  storedAccent,
  storeAccent,
} from "./accent";
import { memoryStore } from "./storage";

function rootStyle(): CSSStyleDeclaration {
  return document.documentElement.style;
}

describe("accent color", () => {
  beforeEach(() => {
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.removeAttribute("style");
  });

  afterEach(() => {
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.removeAttribute("style");
  });

  it("uses the existing desktop blue by default", () => {
    expect(DEFAULT_ACCENT).toBe("blue");
    expect(normalizeAccent(null)).toBe("blue");
    expect(normalizeAccent("not-a-colour")).toBe("blue");
    expect(normalizeAccent("violet")).toBe("violet");
    expect(ACCENTS.map((option) => option.id)).toContain("teal");
  });

  it("repaints the brand ramp for the resolved theme", () => {
    applyAccent("blue", "light");
    expect(rootStyle().getPropertyValue("--color-brand")).toBe("#3b6ef0");
    expect(rootStyle().getPropertyValue("--color-brand-hover")).toBe("#2f5fdc");
    expect(rootStyle().getPropertyValue("--color-brand-soft")).toBe(
      "rgba(59, 110, 240, 0.1)",
    );
    expect(rootStyle().getPropertyValue("--color-focus")).toBe(
      "rgba(59, 110, 240, 0.35)",
    );
    expect(rootStyle().getPropertyValue("--color-accent")).toBe("#3b6ef0");

    applyAccent("blue", "dark");
    expect(rootStyle().getPropertyValue("--color-brand")).toBe("#6f96ff");
    expect(rootStyle().getPropertyValue("--color-brand-hover")).toBe("#8aa9ff");
    expect(rootStyle().getPropertyValue("--color-brand-soft")).toBe(
      "rgba(111, 150, 255, 0.16)",
    );
    expect(rootStyle().getPropertyValue("--color-focus")).toBe(
      "rgba(111, 150, 255, 0.45)",
    );
  });

  it("derives translucent tints from the selected hue", () => {
    applyAccent("violet", "dark");
    expect(rootStyle().getPropertyValue("--color-brand")).toBe("#b58cf5");
    expect(rootStyle().getPropertyValue("--color-brand-soft")).toBe(
      "rgba(181, 140, 245, 0.16)",
    );
    expect(rootStyle().getPropertyValue("--color-focus")).toBe(
      "rgba(181, 140, 245, 0.45)",
    );
  });

  it("persists a selection and restores it on the next read", () => {
    const store = memoryStore();
    expect(storedAccent(store)).toBe("blue");

    storeAccent("teal", store);
    expect(store.getItem(ACCENT_STORAGE_KEY)).toBe("teal");
    expect(storedAccent(store)).toBe("teal");
    expect(rootStyle().getPropertyValue("--color-brand")).toBe("#0f766e");
  });

  it("ignores an unknown stored value", () => {
    const store = memoryStore({ [ACCENT_STORAGE_KEY]: "chartreuse" });
    expect(storedAccent(store)).toBe("blue");
  });
});
