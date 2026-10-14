import { describe, expect, it } from "vitest";
import { parseUnifiedDiff } from "./DiffView";

describe("parseUnifiedDiff", () => {
  it("parses hunks, context, adds and deletes with line numbers", () => {
    const lines = parseUnifiedDiff(
      [
        "--- a/main.rs",
        "+++ b/main.rs",
        "@@ -1,3 +1,4 @@",
        " fn main() {",
        "-    println!(\"old\");",
        "+    println!(\"new\");",
        "+    helper();",
        " }",
      ].join("\n"),
    );
    expect(lines).toEqual([
      { kind: "hunk", oldNo: null, newNo: null, text: "@@ -1,3 +1,4 @@" },
      { kind: "ctx", oldNo: 1, newNo: 1, text: "fn main() {" },
      { kind: "del", oldNo: 2, newNo: null, text: "    println!(\"old\");" },
      { kind: "add", oldNo: null, newNo: 2, text: "    println!(\"new\");" },
      { kind: "add", oldNo: null, newNo: 3, text: "    helper();" },
      { kind: "ctx", oldNo: 3, newNo: 4, text: "}" },
    ]);
  });

  it("handles a new-file hunk starting at line 0", () => {
    const lines = parseUnifiedDiff("--- a/x\n+++ b/x\n@@ -0,0 +1,2 @@\n+one\n+two");
    expect(lines[0].kind).toBe("hunk");
    expect(lines[1]).toEqual({ kind: "add", oldNo: null, newNo: 1, text: "one" });
    expect(lines[2]).toEqual({ kind: "add", oldNo: null, newNo: 2, text: "two" });
  });

  it("skips file headers and tolerates empty input", () => {
    expect(parseUnifiedDiff("")).toEqual([]);
    expect(parseUnifiedDiff("--- a/x\n+++ b/x")).toEqual([]);
  });

  it("ignores non-content trailer lines without advancing counters", () => {
    const lines = parseUnifiedDiff(
      [
        "--- a/x",
        "+++ b/x",
        "@@ -1,2 +1,2 @@",
        " one",
        "\\ No newline at end of file",
        "-old",
        "+new",
        "Binary files a/x and b/x differ",
      ].join("\n"),
    );
    expect(lines.map((l) => l.kind)).toEqual(["hunk", "ctx", "del", "add"]);
    expect(lines[1]).toMatchObject({ kind: "ctx", oldNo: 1, newNo: 1 });
    expect(lines[2]).toMatchObject({ kind: "del", oldNo: 2, newNo: null });
    expect(lines[3]).toMatchObject({ kind: "add", oldNo: null, newNo: 2 });
  });

  it("tolerates hunk headers without spaces", () => {
    const lines = parseUnifiedDiff("@@-1,2 +3,4@@\n-old\n+new");
    expect(lines[0].kind).toBe("hunk");
    expect(lines[1]).toMatchObject({ kind: "del", oldNo: 1 });
    expect(lines[2]).toMatchObject({ kind: "add", newNo: 3 });
  });

  it("drops preamble before the first hunk instead of numbering it", () => {
    const lines = parseUnifiedDiff("diff --git a/x b/x\nindex 123..456\n@@ -5,1 +5,1 @@\n-old\n+new");
    expect(lines[0].kind).toBe("hunk");
    expect(lines[1]).toMatchObject({ kind: "del", oldNo: 5 });
    expect(lines[2]).toMatchObject({ kind: "add", newNo: 5 });
  });
});
