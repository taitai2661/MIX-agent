import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatWelcome } from "./ChatWelcome";
import type { Welcome } from "./greeting";

const welcome: Welcome = {
  id: "welcome",
  title: ["今日は", "何を作りますか"],
  titleIds: ["title.1", "title.2"],
  note: "安全に作業できます",
  noteId: "note.1",
  suggestions: [{ id: "s1", label: "調べる", prompt: "これを調べて" }],
  used: [],
};

describe("ChatWelcome", () => {
  it("renders the loading skeleton for an existing conversation", () => {
    const html = renderToStaticMarkup(
      <ChatWelcome loading welcome={null} onPickSuggestion={() => {}} />,
    );
    expect(html).toContain("message-skeleton-list");
    expect(html).toContain('aria-label="読み込み中…"');
    expect(html).not.toContain("welcome-icon");
  });

  it("renders the greeting and suggestions for a new conversation", () => {
    const html = renderToStaticMarkup(
      <ChatWelcome
        loading={false}
        welcome={welcome}
        onPickSuggestion={() => {}}
      />,
    );
    expect(html).toContain("welcome-icon");
    expect(html).toContain("今日は何を作りますか");
    expect(html).toContain("安全に作業できます");
    expect(html).toContain("調べる");
  });

  it("renders nothing when no greeting is available", () => {
    expect(
      renderToStaticMarkup(
        <ChatWelcome
          loading={false}
          welcome={null}
          onPickSuggestion={() => {}}
        />,
      ),
    ).toBe("");
  });
});
