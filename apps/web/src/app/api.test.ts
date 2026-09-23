import { describe, it, expect, vi, afterEach } from "vitest";
import { api, binary, setCSRF } from "./api";
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
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
describe("timeouts and cancellation", () => {
  function abortingFetch() {
    return vi.fn(
      (_url: string, init: any) =>
        new Promise((_, reject) => {
          init.signal.addEventListener("abort", () =>
            reject(new DOMException("Aborted", "AbortError")),
          );
        }),
    );
  }
  it("aborts a hung request once the timeout fires", async () => {
    vi.useFakeTimers();
    const fetch = abortingFetch();
    vi.stubGlobal("fetch", fetch);
    const promise = api("/slow-resource", "GET", undefined, undefined, {
      timeoutMs: 5,
    });
    const assertion = expect(promise).rejects.toThrow(
      "リクエストがタイムアウトしました",
    );
    await vi.advanceTimersByTimeAsync(5);
    await assertion;
    expect(fetch.mock.calls[0][1].signal.aborted).toBe(true);
  });
  it("early-aborts when the caller's signal is cancelled", async () => {
    const controller = new AbortController();
    vi.stubGlobal("fetch", abortingFetch());
    const promise = api("/x", "GET", undefined, undefined, {
      signal: controller.signal,
    });
    controller.abort(new Error("cancelled"));
    await expect(promise).rejects.toThrow("リクエストを中断しました");
  });
  it("keeps the session cookie on binary uploads that time out", async () => {
    vi.useFakeTimers();
    const fetch = abortingFetch();
    vi.stubGlobal("fetch", fetch);
    const promise = binary("/files/import", {}, { timeoutMs: 5 });
    const assertion = expect(promise).rejects.toThrow(
      "リクエストがタイムアウトしました",
    );
    await vi.advanceTimersByTimeAsync(5);
    await assertion;
    expect(fetch.mock.calls[0][1].credentials).toBe("same-origin");
    expect(fetch.mock.calls[0][1].signal.aborted).toBe(true);
  });
});
