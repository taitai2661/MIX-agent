import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatRunPanels, type ChatRunPanelsProps } from "./ChatRunPanels";
import type { Run } from "./types";

const noop = () => {};

const run = (overrides: Record<string, unknown> = {}): Run =>
  ({
    id: "r1",
    status: "running",
    model_id: null,
    mode: "chat",
    remaining: {
      max_seconds: 600,
      max_steps: 10,
      max_tool_calls: 20,
    },
    budget: {
      max_seconds: 1200,
      max_steps: 20,
      max_tool_calls: 40,
    },
    approvals: [],
    checkpoints: [],
    stagnation: [],
    ...overrides,
  }) as unknown as Run;

const props = (
  overrides: Partial<ChatRunPanelsProps> = {},
): ChatRunPanelsProps => ({
  streamText: "",
  reasoning: "",
  active: false,
  runId: "r1",
  fetchedAt: 0,
  canRetry: false,
  retryDisabled: false,
  onRetry: noop,
  onError: noop,
  onResumed: noop,
  onBudgetResolved: noop,
  ...overrides,
});

describe("ChatRunPanels", () => {
  it("renders reasoning and streamed assistant text", () => {
    const html = renderToStaticMarkup(
      <ChatRunPanels
        {...props({ streamText: "回答です", reasoning: "考え中の要約" })}
      />,
    );
    expect(html).toContain("回答です");
    expect(html).toContain("思考の要約（Provider提供）");
    expect(html).toContain("考え中の要約");
  });

  it("uses status-specific processing messages while no text has arrived", () => {
    const running = renderToStaticMarkup(
      <ChatRunPanels {...props({ active: true, run: run() })} />,
    );
    expect(running).toContain("処理しています…");

    const paused = renderToStaticMarkup(
      <ChatRunPanels
        {...props({ active: true, run: run({ status: "paused" }) })}
      />,
    );
    expect(paused).toContain("ブラウザを手動操作中");

    const waiting = renderToStaticMarkup(
      <ChatRunPanels
        {...props({
          active: true,
          run: run({ status: "waiting_approval" }),
        })}
      />,
    );
    expect(waiting).toContain("操作の承認を待っています");
  });

  it("shows a failed-run reason with a retry action", () => {
    const html = renderToStaticMarkup(
      <ChatRunPanels
        {...props({
          run: run({ status: "failed", reason: "失敗しました" }),
          canRetry: true,
        })}
      />,
    );
    expect(html).toContain("失敗しました");
    expect(html).toContain("再試行");
  });

  it("renders checkpoint and budget-extension recovery panels", () => {
    const interrupted = renderToStaticMarkup(
      <ChatRunPanels
        {...props({
          run: run({
            status: "interrupted",
            checkpoints: [
              {
                id: "c1",
                step: 3,
                tool_count: 2,
                trigger: "auto",
                created_at: "",
              },
            ],
          }),
        })}
      />,
    );
    expect(interrupted).toContain("途中再開ポイントから再開");

    const extension = renderToStaticMarkup(
      <ChatRunPanels
        {...props({
          run: run({
            status: "budget_extension_pending",
            budget_extension_request: {
              tool_calls_used: 10,
              tool_calls_limit: 20,
              steps_used: 5,
              steps_limit: 10,
              elapsed_seconds: 30,
              max_seconds: 60,
            },
            budget_extensions_used: 1,
            budget_extensions_max: 3,
          }),
        })}
      />,
    );
    expect(extension).toContain("実行予算の上限に近づいています");
    expect(extension).toContain("延長する");
  });

  it("renders stagnation findings", () => {
    const html = renderToStaticMarkup(
      <ChatRunPanels
        {...props({
          run: run({
            status: "running",
            stagnation: [
              { code: "repeated_tool", tool_id: "search", count: 3 },
            ],
          }),
        })}
      />,
    );
    expect(html).toContain("停滞シグナル");
    expect(html).toContain("repeated_tool");
    expect(html).toContain("search");
    expect(html).toContain("3回");
  });
});
