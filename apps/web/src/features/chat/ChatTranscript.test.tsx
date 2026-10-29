import type { Row } from "@/app/api";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatTranscript, type ChatTranscriptProps } from "./ChatTranscript";

const noop = () => {};

const message = (id: string, role: string, content: string): Row =>
  ({ id, created_at: "2026-01-01T00:00:00Z", data: { role, content } }) as Row;

const props = (
  overrides: Partial<ChatTranscriptProps> = {},
): ChatTranscriptProps => ({
  messages: [message("m1", "user", "こんにちは")],
  callsByMessage: new Map(),
  liveCalls: [],
  active: false,
  runId: "",
  streamText: "",
  reasoning: "",
  fetchedAt: 0,
  editingId: null,
  editText: "",
  savingEdit: false,
  editDisabled: false,
  canRetry: false,
  onEditChange: noop,
  onEditCancel: noop,
  onEditSave: noop,
  onEditStart: noop,
  onRate: noop,
  onApproval: noop,
  onBrowserChange: noop,
  onRetryRun: noop,
  onError: noop,
  onResumed: noop,
  onBudgetResolved: noop,
  ...overrides,
});

describe("ChatTranscript", () => {
  it("renders persisted messages without the scroll shortcut while pinned", () => {
    const html = renderToStaticMarkup(
      <ChatTranscript
        {...props({
          messages: [
            message("m1", "user", "こんにちは"),
            message("m2", "assistant", "返答です"),
          ],
        })}
      />,
    );
    expect(html).toContain('class="messages-wrap"');
    expect(html).toContain("こんにちは");
    expect(html).toContain("返答です");
    expect(html).not.toContain("scroll-to-bottom");
  });

  it("renders live stream text and the current run message", () => {
    const html = renderToStaticMarkup(
      <ChatTranscript
        {...props({
          currentRunMessageId: "m1",
          latestUserMessageId: "m1",
          streamText: "ストリーム中の回答",
          reasoning: "ストリーム中の思考",
        })}
      />,
    );
    expect(html).toContain("ストリーム中の回答");
    expect(html).toContain("ストリーム中の思考");
    expect(html).toContain("こんにちは");
  });
});
