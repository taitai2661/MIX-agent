import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { RunActivity } from "./RunActivity";

describe("RunActivity", () => {
  it("shows a failed call, safe sources, retry state, and collapsed result", () => {
    const html = renderToStaticMarkup(<RunActivity onApproval={() => {}} calls={[{
      id: "call-1", run_id: "run-1", status: "failed", tool_name: "web_search",
      created_at: "2026-08-31T10:00:00+00:00", failure: "Tool failed",
      result: { error: "Tool failed" }, retry: { available: false, label: "個別再実行は未対応です。" },
      result_activity: { sources: [{ host: "example.com", url: "https://example.com/a" }] },
    }]} />);
    expect(html).toContain("失敗");
    expect(html).toContain("Tool failed");
    expect(html).toContain('href="https://example.com/a"');
    expect(html).toContain("詳細");
    expect(html).toContain("1件のソース");
  });

  it("surfaces the retry guidance only when a resume is possible", () => {
    const html = renderToStaticMarkup(<RunActivity onApproval={() => {}} calls={[{
      id: "call-3", run_id: "run-1", status: "unknown", tool_name: "run_terminal",
      created_at: "2026-08-31T10:00:00+00:00", retry: { available: true, label: "外部状態を確認後、この実行を再開できます" },
    }]} />);
    expect(html).toContain("外部状態を確認後、この実行を再開できます");
  });

  it("renders pending approval controls in the inline card", () => {
    const html = renderToStaticMarkup(<RunActivity onApproval={() => {}} calls={[{
      id: "call-2", run_id: "run-1", status: "waiting_approval", tool_name: "write_file",
      created_at: "2026-08-31T10:00:00+00:00", retry: { available: false, label: "新規メッセージで再依頼してください。" },
      approval: { id: "approval-1", status: "pending" },
    }]} />);
    expect(html).toContain("承認待ち");
    expect(html).toContain("今回のみ");
    expect(html).toContain("常に許可");
    expect(html).toContain('class="run-activity-details" open');
  });

  it("collapses the whole run into one summary with a flat row per call", () => {
    const html = renderToStaticMarkup(<RunActivity onApproval={() => {}} calls={[
      { id: "call-1", run_id: "run-1", status: "completed", tool_name: "search", created_at: "2026-08-31T10:00:00+00:00", retry: { available: false, label: "再依頼してください。" } },
      { id: "call-2", run_id: "run-1", status: "running", tool_name: "fetch", created_at: "2026-08-31T10:01:00+00:00", retry: { available: false, label: "実行中です。" } },
    ]} />);
    expect((html.match(/class="run-step /g) || []).length).toBe(2);
    expect((html.match(/class="run-activity-summary"/g) || []).length).toBe(1);
    expect(html).not.toContain("run-step-status");
    expect(html).toContain("sr-only");
    expect(html).toContain("成功");
    expect(html).toContain("実行中");
    expect(html).toContain("ツール実行履歴");
  });

  it("collapses itself once the run has finished", () => {
    const html = renderToStaticMarkup(<RunActivity onApproval={() => {}} calls={[
      { id: "call-1", run_id: "run-1", status: "failed", tool_name: "search", created_at: "2026-08-31T10:00:00+00:00", failure: "Tool failed", retry: { available: false, label: "" } },
    ]} />);
    expect(html).not.toContain('class="run-activity-details" open');
  });

  it("stays expanded while the run is active", () => {
    const html = renderToStaticMarkup(<RunActivity running onApproval={() => {}} calls={[
      { id: "call-1", run_id: "run-1", status: "running", tool_name: "search", created_at: "2026-08-31T10:00:00+00:00", retry: { available: false, label: "" } },
    ]} />);
    expect(html).toContain('class="run-activity-details" open');
  });

  it("stays hidden until a tool call exists, even while the run is active", () => {
    const html = renderToStaticMarkup(<RunActivity running onApproval={() => {}} calls={[]} />);
    expect(html).toBe("");
  });

  it("auto-opens and narrates the live state while a run is active", () => {
    const html = renderToStaticMarkup(<RunActivity running onApproval={() => {}} calls={[{
      id: "call-live", run_id: "", status: "running", tool_name: "web_search",
      created_at: "2026-08-31T10:00:00+00:00", activity: { icon: "search", label: "Webを検索中", detail: "AIニュース" },
      retry: { available: false, label: "" },
    }]} />);
    expect(html).toContain("run-activity is-running");
    expect(html).toContain("Webを検索中");
    expect(html).toContain("AIニュース");
  });
});
