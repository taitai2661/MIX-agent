import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { api } from "@/lib/api";
import {
  DEFAULT_SETTINGS,
  SETTINGS_QUERY_KEY,
  useApplySettings,
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
});
