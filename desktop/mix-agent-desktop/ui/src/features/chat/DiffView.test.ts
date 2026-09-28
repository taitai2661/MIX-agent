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
});
