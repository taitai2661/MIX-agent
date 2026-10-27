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
  it("sends setup POST when randomUUID is unavailable", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: async () => ({ ok: true }) });
    vi.stubGlobal("fetch", fetch);
    vi.stubGlobal("crypto", {
      getRandomValues: (bytes: Uint8Array) => bytes.fill(0),
    });
    await api("/setup/admin", "POST", {
      username: "user",
      password: "test-password",
    });
    expect(fetch).toHaveBeenCalledOnce();
    expect(fetch.mock.calls[0][1].headers["Idempotency-Key"]).toBe(
      "00000000-0000-4000-8000-000000000000",
    );
  });
  it("sends setup POST when Web Crypto is unavailable", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: async () => ({ ok: true }) });
    vi.stubGlobal("fetch", fetch);
    vi.stubGlobal("crypto", undefined);
    await api("/setup/admin", "POST", {
      username: "user",
      password: "test-password",
    });
    expect(fetch).toHaveBeenCalledOnce();
    expect(fetch.mock.calls[0][1].headers["Idempotency-Key"]).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
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
    const assertion =
      expect(promise).rejects.toThrow("リクエストがタイムアウトしました");
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
    const assertion =
      expect(promise).rejects.toThrow("リクエストがタイムアウトしました");
    await vi.advanceTimersByTimeAsync(5);
    await assertion;
    expect(fetch.mock.calls[0][1].credentials).toBe("same-origin");
    expect(fetch.mock.calls[0][1].signal.aborted).toBe(true);
  });
});

describe("automatic 4-retry mechanism", () => {
  it("retries up to 4 times on transient 5xx errors and succeeds when a retry succeeds", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    const fetch = vi.fn().mockImplementation(async () => {
      callCount++;
      if (callCount < 4) {
        return {
          ok: false,
          status: 503,
          headers: new Headers(),
          json: async () => ({ detail: "一時的なエラー" }),
        };
      }
      return {
        ok: true,
        status: 200,
        json: async () => ({ result: "success" }),
      };
    });
    vi.stubGlobal("fetch", fetch);

    const promise = api("/test-retry", "GET", undefined, undefined, {
      retryDelayMs: 10,
    });
    // Advance timers for 3 retries (each retry waits ~10ms, 20ms, 40ms)
    await vi.advanceTimersByTimeAsync(200);
    const result = await promise;

    expect(result).toEqual({ result: "success" });
    expect(callCount).toBe(4);
  });

  it("exhausts 4 retries (5 total requests) and throws last error when all attempts fail", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    const fetch = vi.fn().mockImplementation(async () => {
      callCount++;
      return {
        ok: false,
        status: 500,
        headers: new Headers(),
        json: async () => ({ detail: "サーバーエラー発生" }),
      };
    });
    vi.stubGlobal("fetch", fetch);

    const promise = api("/test-fail", "GET", undefined, undefined, {
      retryDelayMs: 10,
    });
    const assertion = expect(promise).rejects.toThrow("サーバーエラー発生");
    await vi.advanceTimersByTimeAsync(1000);
    await assertion;

    // 1 initial attempt + 4 retries = 5 total calls
    expect(callCount).toBe(5);
  });

  it("does not retry permanent client errors like 400 or 404", async () => {
    let callCount = 0;
    const fetch = vi.fn().mockImplementation(async () => {
      callCount++;
      return {
        ok: false,
        status: 400,
        headers: new Headers(),
        json: async () => ({ detail: "無効なリクエスト" }),
      };
    });
    vi.stubGlobal("fetch", fetch);

    await expect(api("/bad-req", "GET")).rejects.toThrow("無効なリクエスト");
    expect(callCount).toBe(1);
  });

  it("maintains the same Idempotency-Key across retries for POST requests", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    const keysSent: string[] = [];
    const fetch = vi
      .fn()
      .mockImplementation(async (_url: string, init: RequestInit) => {
        callCount++;
        const headers = init.headers as Record<string, string>;
        keysSent.push(headers["Idempotency-Key"]);
        if (callCount < 3) {
          return {
            ok: false,
            status: 502,
            headers: new Headers(),
            json: async () => ({ detail: "Bad Gateway" }),
          };
        }
        return {
          ok: true,
          status: 200,
          json: async () => ({ ok: true }),
        };
      });
    vi.stubGlobal("fetch", fetch);

    const promise = api(
      "/post-endpoint",
      "POST",
      { data: 123 },
      "custom-idempotency-key",
      {
        retryDelayMs: 10,
        retryUnsafe: true,
      },
    );
    await vi.advanceTimersByTimeAsync(200);
    const result = await promise;

    expect(result).toEqual({ ok: true });
    expect(callCount).toBe(3);
    expect(keysSent).toEqual([
      "custom-idempotency-key",
      "custom-idempotency-key",
      "custom-idempotency-key",
    ]);
  });

  it("does not retry a mutating request unless it opts in", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    const fetch = vi.fn().mockImplementation(async () => {
      callCount++;
      return {
        ok: false,
        status: 503,
        headers: new Headers(),
        json: async () => ({ detail: "Service Unavailable" }),
      };
    });
    vi.stubGlobal("fetch", fetch);

    const promise = api("/providers", "POST", { name: "p" }, "key", { retryDelayMs: 10 });
    await expect(promise).rejects.toThrow("Service Unavailable");
    expect(callCount).toBe(1);
  });

  it("honors Retry-After header on 429 rate limit", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    const fetch = vi.fn().mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        const headers = new Headers();
        headers.set("retry-after", "2");
        return {
          ok: false,
          status: 429,
          headers,
          json: async () => ({ detail: "レート制限中" }),
        };
      }
      return {
        ok: true,
        status: 200,
        json: async () => ({ ok: true }),
      };
    });
    vi.stubGlobal("fetch", fetch);

    const promise = api("/rate-limited", "GET");
    // After 1 second, it should still be waiting
    await vi.advanceTimersByTimeAsync(1000);
    expect(callCount).toBe(1);
    // After another 1.1 seconds (2.1s total >= 2s), retry fires
    await vi.advanceTimersByTimeAsync(1100);
    const res = await promise;
    expect(res).toEqual({ ok: true });
    expect(callCount).toBe(2);
  });

  it("retries network failure (fetch throws TypeError)", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    const fetch = vi.fn().mockImplementation(async () => {
      callCount++;
      if (callCount < 3) {
        throw new TypeError("Failed to fetch");
      }
      return {
        ok: true,
        status: 200,
        json: async () => ({ reconnected: true }),
      };
    });
    vi.stubGlobal("fetch", fetch);

    const promise = api("/network-drop", "GET", undefined, undefined, {
      retryDelayMs: 10,
    });
    await vi.advanceTimersByTimeAsync(200);
    const result = await promise;

    expect(result).toEqual({ reconnected: true });
    expect(callCount).toBe(3);
  });

  it("retries binary upload on 500 error", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    const fetch = vi.fn().mockImplementation(async () => {
      callCount++;
      if (callCount < 2) {
        return {
          ok: false,
          status: 500,
          headers: new Headers(),
          json: async () => ({ detail: "アップロード失敗" }),
        };
      }
      return {
        ok: true,
        status: 200,
        blob: async () => new Blob(),
      };
    });
    vi.stubGlobal("fetch", fetch);

    const promise = binary("/upload", {}, { retryDelayMs: 10, retryUnsafe: true });
    await vi.advanceTimersByTimeAsync(100);
    const result = await promise;

    expect(result.ok).toBe(true);
    expect(callCount).toBe(2);
  });
});
