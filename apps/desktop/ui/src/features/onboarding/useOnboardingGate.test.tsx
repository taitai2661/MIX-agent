import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { setLanguage } from "@/lib/i18n";
import { api } from "@/lib/api";
import { DEFAULT_SETTINGS } from "@/lib/settings";
import type { ProviderView, SettingsView } from "@/lib/types";
import { useUi } from "@/lib/ui";
import { useOnboardingGate } from "./useOnboardingGate";

vi.mock("@/lib/api", () => ({
  api: {
    listProviders: vi.fn(async () => []),
    getSettings: vi.fn(async () => ({ ...DEFAULT_SETTINGS })),
    updateSettings: vi.fn(async (input: SettingsView) => input),
  },
}));

const mockedApi = vi.mocked(api);

// jsdom reports en-US; pin the language so labels are deterministic.
beforeAll(() => setLanguage("ja"));

const PROVIDER: ProviderView = {
  id: "pv1",
  kind: "openai",
  label: "OpenAI",
  base_url: "https://api.openai.com/v1",
  enabled: true,
  has_api_key: true,
  preset: "openai",
};

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

function renderGate() {
  return renderHook(() => useOnboardingGate(), { wrapper });
}

/** Resolves once the gate has stopped loading, so assertions are not racing it. */
async function settled(hook: ReturnType<typeof renderGate>) {
  await waitFor(() => expect(hook.result.current.ready).toBe(true));
  return hook;
}

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  useUi.setState({ onboardingForcedOpen: false });
  mockedApi.listProviders.mockResolvedValue([]);
  mockedApi.getSettings.mockResolvedValue({ ...DEFAULT_SETTINGS });
  mockedApi.updateSettings.mockImplementation(async (input) => input);
});

describe("useOnboardingGate", () => {
  it("shows on a fresh install: no provider, no completed flag", async () => {
    const { result } = await settled(renderGate());
    expect(result.current.show).toBe(true);
  });

  it("stays undecided while the provider list is in flight", () => {
    let release: (value: ProviderView[]) => void = () => undefined;
    mockedApi.listProviders.mockReturnValue(
      new Promise<ProviderView[]>((resolve) => {
        release = resolve;
      }),
    );
    const { result } = renderGate();
    // Nothing rendered on a guess: "no data" is not "no providers".
    expect(result.current.ready).toBe(false);
    expect(result.current.show).toBe(false);
    release([]);
  });

  it("does not show when a provider already exists", async () => {
    mockedApi.listProviders.mockResolvedValue([PROVIDER]);
    const { result } = await settled(renderGate());
    expect(result.current.show).toBe(false);
  });

  it("does not show once the user has finished or deferred", async () => {
    mockedApi.getSettings.mockResolvedValue({ ...DEFAULT_SETTINGS, onboarding_completed: true });
    const { result } = await settled(renderGate());
    expect(result.current.show).toBe(false);
  });

  it("reopens from Settings even when a provider exists", async () => {
    mockedApi.listProviders.mockResolvedValue([PROVIDER]);
    mockedApi.getSettings.mockResolvedValue({ ...DEFAULT_SETTINGS, onboarding_completed: true });
    const { result } = await settled(renderGate());
    expect(result.current.show).toBe(false);

    act(() => result.current.reopen());
    await waitFor(() => expect(result.current.show).toBe(true));

    act(() => result.current.complete());
    await waitFor(() => expect(result.current.show).toBe(false));
  });

  it("persists completion rather than only mutating the cache", async () => {
    const { result } = await settled(renderGate());

    act(() => result.current.complete());

    await waitFor(() => expect(mockedApi.updateSettings).toHaveBeenCalledTimes(1));
    const written = mockedApi.updateSettings.mock.calls[0][0];
    expect(written.onboarding_completed).toBe(true);
    // The document is replaced wholesale on save, so the rest of it has to
    // survive — otherwise completing the wizard wipes the user's other settings.
    expect(written.egress_allowlist).toEqual([]);
    expect(written.language).toBeNull();
  });

  it("survives a settings read that fails, rather than blocking the app forever", async () => {
    mockedApi.getSettings.mockRejectedValue("no such command");
    const { result } = await settled(renderGate());
    // Treated as "never onboarded": the wizard is the safe answer when the flag
    // is unknown, and Settings is one click away for anyone who wants out.
    expect(result.current.show).toBe(true);
  });
});