import { describe, expect, it } from "vitest";
import { entryMatchesScope, splitDiffHunks } from "./ChangesPanel";

describe("splitDiffHunks", () => {
  const block = [
    "a/file.ts b/file.ts",
    "index 1111111..2222222 100644",
    "--- a/file.ts",
    "+++ b/file.ts",
    "@@ -1,2 +1,2 @@",
    "-old",
    "+new",
    " context",
    "@@ -10,1 +10,2 @@",
    " context",
    "+added",
  ].join("\n");

  it("keeps one shared header and one body per hunk", () => {
    const { header, hunks } = splitDiffHunks(block);
    expect(header.startsWith("diff --git a/file.ts b/file.ts")).toBe(true);
    expect(hunks).toHaveLength(2);
    expect(hunks[0]?.startsWith("@@ -1,2 +1,2 @@")).toBe(true);
    expect(hunks[1]).toContain("+added");
  });

  it("returns a header-only block when there is no hunk", () => {
    const { header, hunks } = splitDiffHunks("a/x b/x\nBinary files differ");
    expect(header).toContain("Binary files differ");
    expect(hunks).toEqual([]);
  });
});

describe("ChangesPanel scope filter", () => {
  it("lists a file only on the side of git's XY pair that changed", () => {
    // Staged change, clean worktree.
    expect(
      entryMatchesScope(
        { status: "M", index_status: "M", worktree_status: " " },
        "staged",
      ),
    ).toBe(true);
    expect(
      entryMatchesScope(
        { status: "M", index_status: "M", worktree_status: " " },
        "unstaged",
      ),
    ).toBe(false);

    // Unstaged change, clean index.
    expect(
      entryMatchesScope(
        { status: "M", index_status: " ", worktree_status: "M" },
        "unstaged",
      ),
    ).toBe(true);
    expect(
      entryMatchesScope(
        { status: "M", index_status: " ", worktree_status: "M" },
        "staged",
      ),
    ).toBe(false);
  });

  it("lists a partially staged change in both scopes", () => {
    const entry = { status: "MM", index_status: "M", worktree_status: "M" };
    expect(entryMatchesScope(entry, "staged")).toBe(true);
    expect(entryMatchesScope(entry, "unstaged")).toBe(true);
  });

  it("treats untracked files as unstaged, never staged", () => {
    const entry = { status: "??", index_status: "?", worktree_status: "?" };
    expect(entryMatchesScope(entry, "unstaged")).toBe(true);
    expect(entryMatchesScope(entry, "staged")).toBe(false);
  });

  it("falls back to the trimmed status for older backend payloads", () => {
    // No index/worktree fields: both letters are considered.
    expect(entryMatchesScope({ status: "M" }, "staged")).toBe(true);
    expect(entryMatchesScope({ status: "M" }, "unstaged")).toBe(true);
    expect(entryMatchesScope({ status: "??" }, "staged")).toBe(false);
    expect(entryMatchesScope({ status: "??" }, "unstaged")).toBe(true);
  });
});
