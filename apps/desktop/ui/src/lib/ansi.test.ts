import { describe, expect, it } from "vitest";
import { normalizeTerminalText, parseAnsi, stripAnsi } from "./ansi";

describe("ansi", () => {
  it("maps the 8 basic foreground codes to semantic classes", () => {
    const spans = parseAnsi("\x1b[31mred\x1b[32mgreen\x1b[0m plain");
    expect(spans.map((s) => s.text)).toEqual(["red", "green", " plain"]);
    expect(spans[0]?.className).toContain("text-danger");
    expect(spans[1]?.className).toContain("text-success");
    expect(spans[2]?.className).toBe("");
  });

  it("keeps emphasis and combines it with colour", () => {
    const spans = parseAnsi("\x1b[1;34mbold blue\x1b[22m normal");
    expect(spans[0]?.className).toContain("font-bold");
    expect(spans[0]?.className).toContain("text-brand");
    expect(spans[1]?.className).not.toContain("font-bold");
  });

  it("strips cursor and erase sequences from a progress bar", () => {
    // A spinner rewrites the same line with CR; the visible text must not
    // contain the escape bytes, and the line keeps its latest contents.
    const raw = "\x1b[2Kdownloading\r 10%\x1b[1A";
    expect(stripAnsi(raw)).not.toContain("\x1b");
    expect(normalizeTerminalText(raw)).toBe(" 10%");
  });

  it("strips private-mode CSI that the old whitelist missed", () => {
    // Hide cursor, alternate screen, bracketed paste: all common in real PTY
    // output and none of them end in a byte the old parser recognised.
    const raw =
      "\x1b[?25l\x1b[?1049h\x1b[?2004hvisible\x1b[?2004l\x1b[?1049l\x1b[?25h";
    expect(normalizeTerminalText(raw)).toBe("visible");
    expect(stripAnsi(raw)).toBe("visible");
  });

  it("strips OSC titles (BEL and ST terminated)", () => {
    const bel = "\x1b]0;my title\x07ok";
    const st = "\x1b]2;other\x1b\\ok";
    expect(normalizeTerminalText(bel)).toBe("ok");
    expect(normalizeTerminalText(st)).toBe("ok");
    expect(stripAnsi(bel)).toBe("ok");
  });

  it("consumes extended colour forms without treating their numbers as attributes", () => {
    const spans = parseAnsi("\x1b[38;5;196mred-ish\x1b[0m normal");
    expect(spans.map((s) => s.text)).toEqual(["red-ish", " normal"]);
    // No palette match: the text stays default-coloured rather than picking a
    // wrong semantic token from `5` or `196`.
    expect(spans[0]?.className).toBe("");
  });

  it("keeps only the last carriage-return segment per line", () => {
    const raw = "step 1\rstep 2\rstep 3\nnext";
    expect(normalizeTerminalText(raw)).toBe("step 3\nnext");
    // CRLF is a normal line break, not an overwrite.
    expect(normalizeTerminalText("a\r\nb")).toBe("a\nb");
  });

  it("returns the plain text when there are no codes", () => {
    expect(parseAnsi("hello")).toEqual([{ text: "hello", className: "" }]);
  });

  it("drops an unterminated OSC and a CSI split at the end of a chunk", () => {
    // Streaming output can end inside a sequence. Bare ESC bytes used to leak
    // into the transcript because the OSC branch required a terminator.
    expect(normalizeTerminalText("\x1b]0;title")).toBe("");
    expect(normalizeTerminalText("\x1b]0;title\nnext")).not.toContain("\x1b");
    expect(normalizeTerminalText("before\x1b[3")).toBe("before");
    expect(stripAnsi("before\x1b[3")).toBe("before");
  });

  it("maps bright, background and reset SGR codes", () => {
    const spans = parseAnsi(
      "\x1b[91;102mbright\x1b[39;49m plain\x1b[3;4;9mstyled\x1b[0m end",
    );
    expect(spans[0]?.className).toContain("text-danger");
    expect(spans[0]?.className).toContain("bg-success-soft");
    expect(spans[1]?.className).toBe("");
    expect(spans[2]?.className).toContain("italic");
    expect(spans[2]?.className).toContain("underline");
    expect(spans[2]?.className).toContain("line-through");
    expect(spans[3]?.className).toBe("");
  });

  it("consumes 48;2 extended background colours", () => {
    const spans = parseAnsi("\x1b[48;2;1;2;3mrgb\x1b[0m normal");
    expect(spans.map((s) => s.text)).toEqual(["rgb", " normal"]);
    expect(spans[0]?.className).toBe("");
  });
});
