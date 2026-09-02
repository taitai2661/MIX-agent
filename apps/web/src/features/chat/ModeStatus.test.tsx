import { describe, it, expect } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ModeStatus, ReasoningSummary } from "./ModeStatus";
import type { Row } from "@/app/api";
const model = (data: Record<string, unknown>) =>
  ({ id: "m", data, created_at: "" }) as Row;
describe("mode availability", () => {
  it("explains plain chat fallback for unknown capabilities", () => {
    const html = renderToStaticMarkup(
      <ModeStatus mode="chat" model={model({})} />,
    );
    expect(html).toContain("思考設定を送信しません");
    expect(html).toContain("Tool Calling対応確認");
  });
  it("uses ordinary inference when native reasoning is unavailable", () => {
    const html = renderToStaticMarkup(
      <ModeStatus
        mode="thinking"
        model={model({ capabilities: { reasoning: true } })}
      />,
    );
    expect(html).toContain("通常推論で thinking を実行します");
  });
  it("respects explicit null overrides and empty preset tools", () => {
    const html = renderToStaticMarkup(
      <ModeStatus
        mode="chat"
        model={model({
          reasoning_control: "openai",
          capabilities: { reasoning: true },
          overrides: { reasoning: null },
        })}
        agent={model({ tool_ids: [] })}
      />,
    );
    expect(html).toContain("思考設定を送信しません");
    expect(html).toContain("このプリセットではツールを使いません");
    expect(html).not.toContain("Tool Calling対応確認");
  });
  it("does not hide a received summary based on composer mode", () => {
    expect(
      renderToStaticMarkup(<ReasoningSummary text="公開された要約" />),
    ).toContain("公開された要約");
    expect(renderToStaticMarkup(<ReasoningSummary text="" />)).toBe("");
  });
});
