import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { TodoPane, normalizeTodos } from "./TodoPane";

describe("TodoPane", () => {
  it("renders statuses, progress, and hides phase markers", () => {
    const html = renderToStaticMarkup(
      <TodoPane
        items={[
          { content: "phase:setup", status: "completed" },
          { content: "編集する", status: "in_progress" },
          { content: "検証", status: "pending" },
        ]}
      />,
    );
    expect(html).toContain('class="todo-panel"');
    expect(html).toContain("1/3");
    expect(html).toContain("編集する");
    expect(html).not.toContain("phase:setup");
    expect(html).toContain("完了");
    expect(html).toContain("進行中");
    expect(html).toContain("未着手");
  });

  it("hides the pane when there is no todo yet", () => {
    expect(renderToStaticMarkup(<TodoPane items={[]} />)).toBe("");
    expect(renderToStaticMarkup(<TodoPane />)).toBe("");
    expect(renderToStaticMarkup(<TodoPane items="junk" />)).toBe("");
  });

  it("drops malformed entries instead of failing", () => {
    expect(normalizeTodos("junk")).toEqual([]);
    expect(normalizeTodos([{ content: "x", status: "done" }])).toEqual([]);
    expect(normalizeTodos([{ content: "  ", status: "pending" }, null, 7])).toEqual([]);
    expect(normalizeTodos([{ content: "ok", status: "pending" }])).toEqual([
      { content: "ok", status: "pending" },
    ]);
  });
});
