import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { api } from "@/lib/api";
import { useToasts } from "@/lib/toast";
import {
  DEFAULT_SETTINGS,
  SETTINGS_QUERY_KEY,
  resetSettingsBaseline,
  useApplySettings,
  useSettingsAutosave,
  useSettingsForm,
  useSettingsSaveStatus,
} from "./settings";
import type { SettingsView } from "./types";

vi.mock("@/lib/api", () => ({
  api: {
    getSettings: vi.fn(),
    updateSettings: vi.fn(),
  },
}));

const mockedApi = vi.mocked(api);

/** Observe the autosave debounce instead of guessing how long it takes.
 *
 * Returns how many debounces have fired and which are still pending. Waiting
 * for those values gives a deterministic signal that the debounce window has
 * elapsed and that no further write is scheduled.
 */
function trackDebounce(delay: number) {
  const nativeSetTimeout = window.setTimeout.bind(window);
  const nativeClearTimeout = window.clearTimeout.bind(window);
  const state = { active: new Set<number>(), fired: 0 };

  vi.spyOn(window, "setTimeout").mockImplementation(((
    handler: TimerHandler,
    timeout?: number,
    ...args: unknown[]
  ) => {
    const id = nativeSetTimeout(
      (...callbackArgs: unknown[]) => {
        state.active.delete(id);
        if (typeof handler === "function") {
          (handler as (...a: unknown[]) => void)(...callbackArgs);
        }
        if (timeout === delay) state.fired += 1;
      },
      timeout,
      ...args,
    ) as number;
    if (timeout === delay) state.active.add(id);
    return id;
  }) as typeof window.setTimeout);

  vi.spyOn(window, "clearTimeout").mockImplementation(((id?: number) => {
    if (id !== undefined) state.active.delete(id);
    nativeClearTimeout(id);
  }) as typeof window.clearTimeout);

  return state;
}

function wrapper(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
  };
}

describe("useApplySettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetSettingsBaseline();
    mockedApi.updateSettings.mockImplementation(async (input) => input);
  });

  it("writes the persisted row, not unrelated unsaved form edits", async () => {
    const persisted: SettingsView = { ...DEFAULT_SETTINGS, theme: "light" };
    mockedApi.getSettings.mockResolvedValue(persisted);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    // The Settings form keeps its draft in this cache entry.
    client.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, {
      ...persisted,
      verify_command: "half-typed draft",
    });
    const saved = vi.fn();
    window.addEventListener("mix:settings-saved", saved);

    const { result } = renderHook(() => useApplySettings(), {
      wrapper: wrapper(client),
    });
    await result.current({ theme: "dark" });

    expect(mockedApi.updateSettings).toHaveBeenCalledWith({
      ...persisted,
      theme: "dark",
    });
    // The draft is still in the form, so it must remain marked as unsaved.
    expect(
      client.getQueryData<SettingsView>(SETTINGS_QUERY_KEY)?.verify_command,
    ).toBe("half-typed draft");
    expect(
      (saved.mock.calls[0]?.[0] as CustomEvent<SettingsView>).detail
        .verify_command,
    ).toBeNull();

    window.removeEventListener("mix:settings-saved", saved);
  });

  it("keeps a draft dirty across an unmount/remount", async () => {
    const persisted: SettingsView = { ...DEFAULT_SETTINGS };
    mockedApi.getSettings.mockResolvedValue(persisted);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapperComponent = wrapper(client);

    const first = renderHook(() => useSettingsForm(), {
      wrapper: wrapperComponent,
    });
    await waitFor(() => expect(first.result.current.loaded).toBe(true));
    act(() => first.result.current.patch({ verify_command: "draft" }));
    await waitFor(() => expect(first.result.current.dirty).toBe(true));
    first.unmount();

    // A fresh observer is exactly what the Settings pane creates when the user
    // comes back from another view: it must see the draft as unsaved, not
    // re-baseline it as the saved row.
    const second = renderHook(() => useSettingsForm(), {
      wrapper: wrapperComponent,
    });
    await waitFor(() => expect(second.result.current.loaded).toBe(true));
    expect(second.result.current.data.verify_command).toBe("draft");
    expect(second.result.current.dirty).toBe(true);
  });
});

describe("useSettingsAutosave", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetSettingsBaseline();
    useToasts.setState({ toasts: [] });
    useSettingsSaveStatus.setState({ status: "idle" });
    mockedApi.updateSettings.mockImplementation(async (input) => input);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("persists the latest draft after the debounce and offers undo", async () => {
    const persisted: SettingsView = { ...DEFAULT_SETTINGS };
    mockedApi.getSettings.mockResolvedValue(persisted);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const { result } = renderHook(
      () => ({ form: useSettingsForm(), autosave: useSettingsAutosave(10) }),
      { wrapper: wrapper(client) },
    );

    await waitFor(() => expect(result.current.form.loaded).toBe(true));
    act(() => result.current.form.patch({ verify_command: "npm test" }));

    await waitFor(() =>
      expect(mockedApi.updateSettings).toHaveBeenCalledTimes(1),
    );
    expect(mockedApi.updateSettings).toHaveBeenCalledWith(
      expect.objectContaining({ verify_command: "npm test" }),
    );
    // A successful write clears the dirty flag; the sidebar/header can stop
    // saying "saving…".
    await waitFor(() => expect(result.current.form.dirty).toBe(false));

    // The undo action restores the row that was persisted before this write.
    const toast = useToasts.getState().toasts[0];
    expect(toast?.id).toBe("settings-autosave");
    act(() => toast?.action?.onClick());
    await waitFor(() =>
      expect(mockedApi.updateSettings).toHaveBeenCalledTimes(2),
    );
    expect(mockedApi.updateSettings).toHaveBeenLastCalledWith(persisted);
    await waitFor(() =>
      expect(
        client.getQueryData<SettingsView>(SETTINGS_QUERY_KEY)?.verify_command,
      ).toBeNull(),
    );
  });

  it("adopts a server-normalised value instead of re-saving forever", async () => {
    const debounce = trackDebounce(10);
    const persisted: SettingsView = { ...DEFAULT_SETTINGS };
    mockedApi.getSettings.mockResolvedValue(persisted);
    // The backend clamps the TTL; the draft keeps the raw number.
    mockedApi.updateSettings.mockImplementation(async (input) => ({
      ...input,
      approval_ttl_minutes: 1440,
    }));
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const { result } = renderHook(
      () => ({ form: useSettingsForm(), autosave: useSettingsAutosave(10) }),
      { wrapper: wrapper(client) },
    );

    await waitFor(() => expect(result.current.form.loaded).toBe(true));
    act(() => result.current.form.patch({ approval_ttl_minutes: 9999 }));

    await waitFor(() =>
      expect(mockedApi.updateSettings).toHaveBeenCalledTimes(1),
    );
    // The debounce actually elapsed, and once the stored row replaced the
    // draft no further debounce is pending. These are the observable signals
    // behind "no second save"; a bare sleep was either too short or wasteful.
    await waitFor(() => expect(debounce.fired).toBeGreaterThanOrEqual(1));
    await waitFor(() => expect(result.current.form.dirty).toBe(false));
    expect(debounce.active.size).toBe(0);
    expect(mockedApi.updateSettings).toHaveBeenCalledTimes(1);
    expect(
      client.getQueryData<SettingsView>(SETTINGS_QUERY_KEY)
        ?.approval_ttl_minutes,
    ).toBe(1440);
  });

  it("merges this window's change onto the latest stored row", async () => {
    const persisted: SettingsView = {
      ...DEFAULT_SETTINGS,
      theme: "light",
    };
    mockedApi.getSettings
      .mockResolvedValueOnce(persisted)
      // Between the form loading and the write another window switched the
      // theme. A full-row snapshot would have reverted it.
      .mockResolvedValue({ ...persisted, theme: "dark" });
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const { result } = renderHook(
      () => ({ form: useSettingsForm(), autosave: useSettingsAutosave(10) }),
      { wrapper: wrapper(client) },
    );

    await waitFor(() => expect(result.current.form.loaded).toBe(true));
    act(() => result.current.form.patch({ verify_command: "pnpm test" }));

    await waitFor(() =>
      expect(mockedApi.updateSettings).toHaveBeenCalledTimes(1),
    );
    expect(mockedApi.updateSettings).toHaveBeenCalledWith(
      expect.objectContaining({ theme: "dark", verify_command: "pnpm test" }),
    );
  });

  it("saves routine toggles without an undo toast", async () => {
    const persisted: SettingsView = { ...DEFAULT_SETTINGS };
    mockedApi.getSettings.mockResolvedValue(persisted);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const { result } = renderHook(
      () => ({ form: useSettingsForm(), autosave: useSettingsAutosave(10) }),
      { wrapper: wrapper(client) },
    );

    await waitFor(() => expect(result.current.form.loaded).toBe(true));
    // Let the baseline effect run before editing; a real user cannot touch a
    // control before the pane has mounted.
    await act(async () => {});
    act(() => result.current.form.patch({ auto_check_updates: false }));

    await waitFor(() =>
      expect(mockedApi.updateSettings).toHaveBeenCalledTimes(1),
    );
    await waitFor(() => expect(result.current.form.dirty).toBe(false));
    expect(useToasts.getState().toasts).toHaveLength(0);
    expect(useSettingsSaveStatus.getState().status).toBe("saved");
  });

  it("offers a retry after a failed write and succeeds on the second try", async () => {
    const persisted: SettingsView = { ...DEFAULT_SETTINGS };
    mockedApi.getSettings.mockResolvedValue(persisted);
    mockedApi.updateSettings.mockRejectedValueOnce(new Error("offline"));
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const { result } = renderHook(
      () => ({ form: useSettingsForm(), autosave: useSettingsAutosave(10) }),
      { wrapper: wrapper(client) },
    );

    await waitFor(() => expect(result.current.form.loaded).toBe(true));
    act(() => result.current.form.patch({ verify_command: "npm test" }));

    await waitFor(() =>
      expect(useSettingsSaveStatus.getState().status).toBe("error"),
    );
    const errorToast = useToasts
      .getState()
      .toasts.find((toast) => toast.id === "settings-autosave-error");
    expect(errorToast?.action?.label).toMatch(/再試行|Retry/);
    // The draft stays dirty so nothing is lost.
    expect(result.current.form.dirty).toBe(true);

    act(() => errorToast?.action?.onClick());
    await waitFor(() =>
      expect(mockedApi.updateSettings).toHaveBeenCalledTimes(2),
    );
    await waitFor(() => expect(result.current.form.dirty).toBe(false));
    expect(useSettingsSaveStatus.getState().status).toBe("saved");
  });

  it("flushes a pending debounce when the page is hidden", async () => {
    const persisted: SettingsView = { ...DEFAULT_SETTINGS };
    mockedApi.getSettings.mockResolvedValue(persisted);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const { result } = renderHook(
      // Long enough that only the flush can have written it.
      () => ({
        form: useSettingsForm(),
        autosave: useSettingsAutosave(10_000),
      }),
      { wrapper: wrapper(client) },
    );

    await waitFor(() => expect(result.current.form.loaded).toBe(true));
    act(() => result.current.form.patch({ verify_command: "npm test" }));
    act(() => window.dispatchEvent(new Event("pagehide")));

    await waitFor(() =>
      expect(mockedApi.updateSettings).toHaveBeenCalledWith(
        expect.objectContaining({ verify_command: "npm test" }),
      ),
    );
  });
});
