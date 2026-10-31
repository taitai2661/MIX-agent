import type { Row } from "@/app/api";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatMessage, type ChatMessageProps } from "./ChatMessage";
import type { ConversationRun } from "./types";

const noop = () => {};

const message = (data: Record<string, unknown>): Row =>
  ({ id: "m1", created_at: "2026-01-01T00:00:00Z", data }) as Row;

const props = (
  overrides: Partial<ChatMessageProps> = {},
): ChatMessageProps => ({
  message: message({ role: "user", content: "こんにちは" }),
  calls: [],
  liveCalls: [],
  active: false,
  runId: "",
  editing: false,
  editText: "",
  savingEdit: false,
  editDisabled: false,
  showTodos: false,
  onEditChange: noop,
  onEditCancel: noop,
  onEditSave: noop,
  onEditStart: noop,
  onRate: noop,
  onApproval: noop,
  onBrowserChange: noop,
  ...overrides,
});

const storedRun = (overrides: Record<string, unknown>) =>
  ({
    id: "r1",
    status: "completed",
    ...overrides,
  }) as unknown as ConversationRun;

describe("ChatMessage", () => {
  it("renders the user label, markdown, edit trigger, and artifact fallback", () => {
    const html = renderToStaticMarkup(
      <ChatMessage
        {...props({
          message: message({
            role: "user",
            content: "こんにちは **world**",
            artifact_ids: ["a1"],
          }),
        })}
      />,
    );
    expect(html).toContain("あなた");
    expect(html).toContain("<strong>world</strong>");
    expect(html).toContain("編集");
    expect(html).toContain("成果物");
    expect(html).toContain('href="/api/v1/artifacts/a1"');
  });

  it("renders assistant metadata, artifacts, performance, and feedback controls", () => {
    const html = renderToStaticMarkup(
      <ChatMessage
        {...props({
          message: message({
            role: "assistant",
            content: "回答です",
            artifacts: [
              {
                artifact_id: "a2",
                name: "report.pdf",
                mime: "application/pdf",
                size: 2048,
              },
            ],
            performance: {
              output_tokens: 20,
              generation_ms: 1000,
              tokens_per_second: 20,
            },
            auto_selection: { model_name: "Test Model", model_id: "m" },
            feedback: "up",
          }),
        })}
      />,
    );
    expect(html).toContain("MIX agent");
    expect(html).toContain("回答です");
    expect(html).toContain("report.pdf");
    expect(html).toContain("20.0 tokens/s");
    expect(html).toContain("Auto: Test Model");
    expect(html).toContain('aria-label="良い回答"');
    expect(html).toContain('aria-label="良くない回答"');
    expect(html).toContain('class="selected"');
  });

  it("switches to the inline editor with keyboard hints", () => {
    const html = renderToStaticMarkup(
      <ChatMessage
        {...props({
          message: message({ role: "user", content: "old" }),
          editing: true,
          editText: "new content",
        })}
      />,
    );
    expect(html).toContain("<textarea");
    expect(html).toContain("new content");
    expect(html).toContain("キャンセル");
    expect(html).toContain("保存");
    expect(html).toMatch(/(⌘|Ctrl)\+Enter で保存/);
    expect(html).not.toContain("message-edit-trigger");
  });

  it("renders context summaries and the todo pane for the current turn", () => {
    const html = renderToStaticMarkup(
      <ChatMessage
        {...props({
          runs: [
            storedRun({
              message_id: "m1",
              context_summary: { text: "古い会話の要約", covered_count: 2 },
            }),
          ],
          showTodos: true,
          todos: [{ content: "調べる", status: "in_progress" }],
        })}
      />,
    );
    expect(html).toContain("この実行で文脈を圧縮しました");
    expect(html).toContain("古い会話の要約");
    expect(html).toContain("調べる");
  });
});
