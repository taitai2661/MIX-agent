import { describe, expect, it } from "vitest";
import { matchSettingsAnchors, SETTINGS_ANCHORS } from "./settingsAnchors";

const identity = (key: string) => key;

describe("matchSettingsAnchors", () => {
  it("finds a field by its Japanese label", () => {
    const hits = matchSettingsAnchors("予算", identity);
    expect(hits.map((hit) => hit.id)).toContain("general-budget");
  });

  it("finds a field by English keywords", () => {
    expect(
      matchSettingsAnchors("api key", identity).map((hit) => hit.id),
    ).toContain("providers-api-key");
    expect(
      matchSettingsAnchors("sqlcipher", identity).map((hit) => hit.id),
    ).toContain("general-db-encryption");
  });

  it("matches the translated label as well as the canonical key", () => {
    const hits = matchSettingsAnchors("budget", (key) =>
      key === "月次予算 (USD)" ? "Monthly budget (USD)" : key,
    );
    expect(hits.map((hit) => hit.id)).toContain("general-budget");
  });

  it("returns nothing for an empty query and caps results", () => {
    expect(matchSettingsAnchors("   ", identity)).toEqual([]);
    expect(matchSettingsAnchors("a", identity, 2)).toHaveLength(2);
  });

  it("keeps ids unique", () => {
    const ids = SETTINGS_ANCHORS.map((anchor) => anchor.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
