import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { ToolHistory } from "./ToolHistory";

describe("ToolHistory", () => {
  it("shows a failed call, safe sources, retry state, and collapsed result", () => {
    const html = renderToStaticMarkup(<ToolHistory onApproval={() => {}} calls={[{
      id: "call-1", run_id: "run-1", status: "failed", tool_name: "web_search",
      created_at: "2026-08-31T10:00:00+00:00", failure: "Tool failed",
      result: { error: "Tool failed" }, retry: { available: false, label: "個別再実行は未対応です。" },
      result_activity: { sources: [{ host: "example.com", url: "https://example.com/a" }] },
    }]} />);
    expect(html).toContain("失敗");
    expect(html).toContain("Tool failed");
    expect(html).toContain('href="https://example.com/a"');
    expect(html).toContain("個別再実行は未対応です。");
    expect(html).toContain("結果の詳細");
  });

  it("renders pending approval controls in the inline card", () => {
    const html = renderToStaticMarkup(<ToolHistory onApproval={() => {}} calls={[{
      id: "call-2", run_id: "run-1", status: "waiting_approval", tool_name: "write_file",
      created_at: "2026-08-31T10:00:00+00:00", retry: { available: false, label: "新規メッセージで再依頼してください。" },
      approval: { id: "approval-1", status: "pending" },
    }]} />);
    expect(html).toContain("承認待ち");
    expect(html).toContain("今回のみ");
    expect(html).toContain("常に許可");
  });
});
