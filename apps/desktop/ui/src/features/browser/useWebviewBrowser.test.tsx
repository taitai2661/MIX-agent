import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api";
import { useWebviewBrowser } from "./useWebviewBrowser";

vi.mock("@/lib/api", () => ({
  api: {
    browserInvoke: vi.fn(async () => ({
      ok: true,
      payload: {
        pages: [],
        active: {},
        can_go_back: false,
        can_go_forward: false,
      },
    })),
    browserStateEvent: vi.fn(),
    browserClearData: vi.fn(async () => undefined),
    browserStop: vi.fn(async () => undefined),
    browserSurfaceBounds: vi.fn(async () => undefined),
  },
}));

const mockedApi = vi.mocked(api);

beforeEach(() => vi.clearAllMocks());

describe("useWebviewBrowser listener lifecycle", () => {
  it("unlistens when registration resolves after unmount", async () => {
    let resolveRegistration!: (unlisten: () => void) => void;
    mockedApi.browserStateEvent.mockImplementation(
      () =>
        new Promise<() => void>((resolve) => {
          resolveRegistration = resolve;
        }),
    );

    const { unmount } = renderHook(() => useWebviewBrowser("ws-1"));
    await waitFor(() => expect(mockedApi.browserStateEvent).toHaveBeenCalled());

    unmount();
    const unlisten = vi.fn();
    resolveRegistration(unlisten);
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("ignores events delivered after unmount", async () => {
    let handler!: (event: { workspace_id: string }) => void;
    let resolveRegistration!: (unlisten: () => void) => void;
    mockedApi.browserStateEvent.mockImplementation(
      (fn: (event: { workspace_id: string }) => void) => {
        handler = fn;
        return new Promise((resolve) => (resolveRegistration = resolve));
      },
    );

    const { unmount } = renderHook(() => useWebviewBrowser("ws-1"));
    await waitFor(() => expect(handler).toBeDefined());
    unmount();
    resolveRegistration(vi.fn());
    await Promise.resolve();

    const before = mockedApi.browserInvoke.mock.calls.length;
    handler({ workspace_id: "ws-1" });
    await Promise.resolve();
    expect(mockedApi.browserInvoke.mock.calls.length).toBe(before);
  });
});
