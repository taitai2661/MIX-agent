import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { api } from "@/lib/api";
import {
  DEFAULT_SETTINGS,
  SETTINGS_QUERY_KEY,
  resetSettingsBaseline,
  useApplySettings,
  useSettingsForm,
} from "./settings";
import type { SettingsView } from "./types";

vi.mock("@/lib/api", () => ({
  api: {
    getSettings: vi.fn(),
    updateSettings: vi.fn(),
  },
}));

const mockedApi = vi.mocked(api);

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
