// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";

import { queryKeys, useSettings } from "./queries";

// React 19 warns unless the test environment opts in to `act`.
(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

function Probe() {
  const settings = useSettings<{ data: { default_model_id: string } }>();
  return <span>{settings.data?.data.default_model_id ?? "loading"}</span>;
}

it("builds parameterised keys without string concatenation", () => {
  expect(queryKeys.messages("c1")).toEqual(["messages", "c1"]);
  expect(queryKeys.runEvents("r1")).toEqual(["run_events", "r1"]);
  expect(queryKeys.run("r1")).toEqual(["run", "r1"]);
  expect(queryKeys.settings).toEqual(["/settings"]);
});

it("reads /settings through the shared hook", async () => {
  const body = { data: { default_model_id: "model-1" } };
  const fetch = vi.fn().mockResolvedValue({
    ok: true,
    status: 200,
    text: async () => JSON.stringify(body),
    json: async () => body,
  });
  vi.stubGlobal("fetch", fetch);

  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  let root: Root | null = null;

  await act(async () => {
    root = createRoot(host);
    root.render(
      <QueryClientProvider client={client}>
        <Probe />
      </QueryClientProvider>,
    );
  });
  await act(async () => {
    await vi.waitFor(() => {
      expect(host.textContent).toContain("model-1");
    });
  });

  expect(fetch).toHaveBeenCalledWith(
    "/api/v1/settings",
    expect.objectContaining({ credentials: "same-origin" }),
  );
  act(() => root?.unmount());
});
