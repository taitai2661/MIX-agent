import { describe, expect, it } from "vitest";
import { greetings, randomGreeting, timeOfDay } from "./greeting";

const at = (hour: number) => new Date(2026, 0, 1, hour);

describe("welcome greeting", () => {
  it("maps hours to a time of day", () => {
    expect(timeOfDay(at(6))).toBe("morning");
    expect(timeOfDay(at(13))).toBe("afternoon");
    expect(timeOfDay(at(19))).toBe("evening");
    expect(timeOfDay(at(3))).toBe("night");
    expect(timeOfDay(at(23))).toBe("night");
  });

  it("selects a greeting for the current time of day", () => {
    expect(greetings.morning).toContain(randomGreeting(() => 0, at(8)));
    expect(greetings.evening).toContain(randomGreeting(() => 0.5, at(19)));
    expect(greetings.night).toContain(randomGreeting(() => 0.999, at(2)));
  });
});
