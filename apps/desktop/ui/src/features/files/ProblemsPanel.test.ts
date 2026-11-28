import { describe, expect, it } from "vitest";
import { extractProblems } from "./ProblemsPanel";

const lsp = (
  path: string,
  diagnostics: Array<Record<string, unknown>>,
) => ({
  tool_name: "lsp",
  result: { action: "diagnostics", path, diagnostics },
});

describe("extractProblems", () => {
  it("reads lsp diagnostics and sorts errors first", () => {
    const problems = extractProblems([
      lsp("/repo/a.ts", [
        { line: 4, column: 2, severity: "warning", message: "unused" },
        { line: 1, column: 1, severity: "error", message: "type error" },
      ]),
    ]);
    expect(problems.map((problem) => problem.message)).toEqual([
      "type error",
      "unused",
    ]);
    expect(problems[0]?.path).toBe("/repo/a.ts");
  });

  it("keeps only the latest diagnostics for each file", () => {
    const problems = extractProblems([
      lsp("/repo/a.ts", [
        { line: 1, severity: "error", message: "old" },
      ]),
      lsp("/repo/b.ts", [
        { line: 2, severity: "info", message: "other file" },
      ]),
      lsp("/repo/a.ts", [
        { line: 3, severity: "warning", message: "new" },
      ]),
    ]);
    expect(problems.map((problem) => problem.message).sort()).toEqual([
      "new",
      "other file",
    ]);
  });

  it("ignores non-lsp calls and malformed result payloads", () => {
    expect(
      extractProblems([
        { tool_name: "terminal", result: { diagnostics: [] } },
        { tool_name: "lsp", result: { action: "definition" } },
        { tool_name: "lsp", result: null },
        { tool_name: "lsp", result: { action: "diagnostics", path: "" } },
      ]),
    ).toEqual([]);
  });
});
