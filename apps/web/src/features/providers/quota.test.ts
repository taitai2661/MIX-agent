import { beforeAll, describe, expect, it } from "vitest";
import { setLanguage } from "@/app/i18n";
import { resetTimeLabel } from "./quota";

beforeAll(() => setLanguage("ja"));

describe("resetTimeLabel", () => {
  it("renders UTC reset boundaries in the requested zone", () => {
    // 2026-01-01T00:00:00Z is 09:00 on the same day in Japan.
    const label = resetTimeLabel(1767225600, "Asia/Tokyo");
    expect(label).toContain("09:00");
    expect(label).toContain("00:00");
    expect(label).toContain("Asia/Tokyo");
    expect(label).toContain("UTC");
  });

  it("rolls the calendar day forward when UTC is still the previous day", () => {
    // 2026-01-01T15:00:00Z is 2026-01-02 00:00 in Japan.
    const label = resetTimeLabel(1767279600, "Asia/Tokyo");
    expect(label).toContain("01/02");
    expect(label).toContain("00:00");
    expect(label).toContain("UTC 15:00");
  });

  it("returns an empty label for an unknown reset time", () => {
    expect(resetTimeLabel(0)).toBe("");
    expect(resetTimeLabel(Number.NaN)).toBe("");
  });
});
