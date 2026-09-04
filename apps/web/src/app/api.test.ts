import { describe, it, expect, vi, afterEach } from "vitest";
import { api, binary, setCSRF } from "./api";
afterEach(() => vi.unstubAllGlobals());
describe("authenticated mutations", () => {
  it("adds CSRF and idempotency headers", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: async () => ({ ok: true }) });
    vi.stubGlobal("fetch", fetch);
    setCSRF("test-csrf");
    await api("/conversations", "POST", {});
    const options = fetch.mock.calls[0][1];
    expect(options.credentials).toBe("same-origin");
    expect(options.headers["x-csrf-token"]).toBe("test-csrf");
    expect(options.headers["Idempotency-Key"]).toBeTruthy();
  });
  it("keeps the login session cookie on binary mutations too", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: async () => ({}) });
    vi.stubGlobal("fetch", fetch);
    await binary("/files/import", {});
    expect(fetch.mock.calls[0][1].credentials).toBe("same-origin");
  });
  it("surfaces actionable server errors", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        json: async () => ({ detail: "承認は決定済みです" }),
      }),
    );
    await expect(
      api("/approvals/x/decision", "POST", { decision: "once" }),
    ).rejects.toThrow("承認は決定済みです");
  });
});
