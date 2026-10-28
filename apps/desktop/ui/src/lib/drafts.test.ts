import { beforeEach, describe, expect, it } from "vitest";
import {
  COMPOSER_DRAFT_TTL_MS,
  clearComposerDraft,
  clearComposerDrafts,
  countComposerDrafts,
  loadComposerDraft,
  pruneComposerDrafts,
  saveComposerDraft,
} from "./drafts";

describe("composer drafts", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("round-trips a draft per slot", () => {
    saveComposerDraft("thread-a", "hello");
    saveComposerDraft("thread-b", "world");
    expect(loadComposerDraft("thread-a")).toBe("hello");
    expect(loadComposerDraft("thread-b")).toBe("world");
    expect(loadComposerDraft("thread-c")).toBe("");
  });

  it("expires old drafts instead of restoring them forever", () => {
    const now = Date.UTC(2026, 0, 1);
    saveComposerDraft("thread-a", "old secret", now);
    expect(loadComposerDraft("thread-a", now + COMPOSER_DRAFT_TTL_MS - 1)).toBe(
      "old secret",
    );
    expect(loadComposerDraft("thread-a", now + COMPOSER_DRAFT_TTL_MS + 1)).toBe(
      "",
    );
    expect(localStorage.getItem("mix:composer-draft:thread-a")).toBeNull();
  });

  it("reads drafts written by an older build", () => {
    localStorage.setItem("mix:composer-draft:legacy", "plain text");
    expect(loadComposerDraft("legacy")).toBe("plain text");
  });

  it("clears one draft without touching the others", () => {
    saveComposerDraft("a", "one");
    saveComposerDraft("b", "two");
    clearComposerDraft("a");
    expect(loadComposerDraft("a")).toBe("");
    expect(loadComposerDraft("b")).toBe("two");
  });

  it("prunes expired drafts on boot", () => {
    const now = Date.UTC(2026, 0, 1);
    saveComposerDraft("old", "old", now);
    saveComposerDraft("fresh", "fresh", now + COMPOSER_DRAFT_TTL_MS);

    expect(pruneComposerDrafts(now + COMPOSER_DRAFT_TTL_MS + 1)).toBe(1);
    expect(loadComposerDraft("old", now + COMPOSER_DRAFT_TTL_MS + 1)).toBe("");
    expect(loadComposerDraft("fresh", now + COMPOSER_DRAFT_TTL_MS + 1)).toBe(
      "fresh",
    );
  });

  it("counts and clears every draft", () => {
    saveComposerDraft("a", "one");
    saveComposerDraft("b", "two");
    localStorage.setItem("unrelated", "keep me");
    expect(countComposerDrafts()).toBe(2);
    expect(clearComposerDrafts()).toBe(2);
    expect(countComposerDrafts()).toBe(0);
    expect(localStorage.getItem("unrelated")).toBe("keep me");
  });
});
