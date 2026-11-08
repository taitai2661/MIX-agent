import { describe, expect, it } from "vitest";

import {
  CONVERSATION_PAGE_SIZE,
  nextConversationOffset,
} from "./conversations";

describe("nextConversationOffset", () => {
  it("advances by one full page", () => {
    expect(nextConversationOffset(CONVERSATION_PAGE_SIZE, 0)).toBe(
      CONVERSATION_PAGE_SIZE,
    );
    expect(nextConversationOffset(CONVERSATION_PAGE_SIZE, 300)).toBe(
      300 + CONVERSATION_PAGE_SIZE,
    );
  });

  it("stops after a short (last) page", () => {
    expect(
      nextConversationOffset(CONVERSATION_PAGE_SIZE - 1, 0),
    ).toBeUndefined();
    expect(nextConversationOffset(0, 200)).toBeUndefined();
  });
});
