import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Statistics } from "./Statistics";

it("renders totals, per-model failures and latency from the statistics view", () => {
  const client = new QueryClient();
  client.setQueryData(["/settings/statistics"], {
    retention_days: 30,
    total: {
      key: "total",
      total: 12,
      success: 9,
      failure: 3,
      failure_rate: 25,
      first_output_ms: 420,
      completion_ms: 3100,
      tokens_per_second: 18.5,
      tps_count: 7,
      classifications: { timeout: 2, rate_limit: 1 },
    },
    groups: [
      {
        key: "p1:m1",
        model_id: "m1",
        model_name: "Test model",
        provider_id: "p1",
        scope: "chat",
        total: 12,
        success: 9,
        failure: 3,
        failure_rate: 25,
        first_output_ms: 420,
        completion_ms: 3100,
        tokens_per_second: 18.5,
        tps_count: 7,
        classifications: { timeout: 2, rate_limit: 1 },
      },
    ],
  });

  const html = renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <Statistics />
    </QueryClientProvider>,
  );

  expect(html).toContain("実行回数");
  expect(html).toContain("12");
  expect(html).toContain("75.0%");
  expect(html).toContain("Test model");
  expect(html).toContain("25.0% 失敗");
  expect(html).toContain("タイムアウト");
  expect(html).toContain("420ms");
});
