import { describe, expect, it } from "vitest";

import { ERROR_CODES_EN, errorCodeText } from "./errorCodes";

describe("stable error code translations", () => {
  it("returns English text for a known code and null otherwise", () => {
    expect(errorCodeText("auth.invalid_credentials")).toBe(
      "Incorrect username or password.",
    );
    expect(errorCodeText("common.backup_in_progress")).toContain("backup");
    expect(errorCodeText("does.not.exist")).toBeNull();
    expect(errorCodeText(undefined)).toBeNull();
  });

  it("keeps the status fallbacks available to dynamic errors", () => {
    for (const status of [
      400, 401, 403, 404, 409, 413, 422, 429, 500, 502, 503,
    ]) {
      expect(ERROR_CODES_EN[`common.http_${status}`]).toBeTruthy();
    }
  });
});
