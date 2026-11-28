import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Guards the choices documented in `docs/desktop/ui-conventions.md`.
 *
 * The design system is only as consistent as the next component allows it to
 * be. These checks catch the three drifts that had actually accumulated:
 *
 *  - a utility naming a colour token that does not exist (`bg-card`,
 *    `text-fg-muted`) — Tailwind silently emits nothing, so the element loses
 *    its background or colour;
 *  - a bare `rounded`, which is Tailwind's 4px default and sits outside the
 *    app's 7 / 10 / 13 / 17 / 22 radius scale;
 *  - a translucent divider (`border-line/60`) and sub-12px text, both of which
 *    the conventions rule out.
 */

const SRC = join(process.cwd(), "src");
const TOKENS_CSS = join(SRC, "styles/tokens.css");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    if (!/\.(ts|tsx)$/.test(entry.name)) return [];
    if (entry.name.includes(".test.")) return [];
    return [path];
  });
}

/** Class tokens live inside quoted strings; comments and prose are ignored. */
function classTokens(source: string): string[] {
  const strings = [...source.matchAll(/["'`]([^"'`]*)["'`]/g)].map(
    (match) => match[1],
  );
  return strings.flatMap((value) => value.split(/\s+/)).filter(Boolean);
}

const tokensCss = readFileSync(TOKENS_CSS, "utf8");
const definedColors = new Set(
  [...tokensCss.matchAll(/--color-([a-z0-9-]+):/g)].map((match) => match[1]),
);

/** Utilities that look like `text-<colour>` but are size/alignment roles. */
const TEXT_ROLES = new Set([
  "3xs",
  "2xs",
  "xs",
  "sm",
  "base",
  "lg",
  "xl",
  "2xl",
  "3xl",
  "4xl",
  "5xl",
  "display",
  "display-sm",
  "start",
  "end",
  "center",
  "left",
  "right",
  "balance",
  "pretty",
  "wrap",
  "nowrap",
  "ellipsis",
  "clip",
]);

/** Non-colour utilities that share a colour prefix. */
const NON_COLOR = new Set([
  "transparent",
  "current",
  "inherit",
  "none",
  "auto",
  "inset",
  "solid",
  "dashed",
  "dotted",
  "double",
  "hidden",
  "visible",
  "collapse",
  "separate",
  "gradient-to-t",
  "gradient-to-tr",
  "gradient-to-r",
  "gradient-to-br",
  "gradient-to-b",
  "gradient-to-bl",
  "gradient-to-l",
  "gradient-to-tl",
  "clip-text",
  "clip-padding",
  "clip-content",
  "clip-border",
  "fixed",
  "local",
  "scroll",
  "cover",
  "contain",
  "bottom",
  "top",
  "center",
  "left",
  "right",
  "x",
  "y",
]);

const COLOR_PREFIX =
  /^(bg|text|border|ring|fill|stroke|divide|from|via|to)-(.+)$/;

/** `border-b-line-strong` → `line-strong`; `ring-offset-bg` → `bg`.
 *  Width-only utilities (`border-b`, `ring-2`) return "" and are skipped. */
function colourName(prefix: string, rest: string): string {
  if (prefix === "border" || prefix === "divide") {
    if (/^[xyseblrt]$/.test(rest)) return "";
    const directional = rest.match(/^[xyseblrt]-(.+)$/);
    if (directional) return directional[1];
  }
  if (prefix === "ring") {
    const offset = rest.match(/^offset-(.+)$/);
    if (offset) return offset[1];
  }
  return rest;
}

describe("design tokens", () => {
  const files = sourceFiles(SRC).map((path) => ({
    path,
    tokens: classTokens(readFileSync(path, "utf8")),
  }));

  it("only uses colour tokens that exist in tokens.css", () => {
    const unknown: string[] = [];
    for (const file of files) {
      for (const token of file.tokens) {
        // Strip the opacity modifier before resolving the name.
        const base = token.replace(/\/\d+$/, "");
        const match = base.match(COLOR_PREFIX);
        if (!match) continue;
        const [, prefix, rest] = match;
        if (!/^[a-z][a-z0-9-]*$/.test(rest)) continue;
        if (prefix === "text" && TEXT_ROLES.has(rest)) continue;
        if (NON_COLOR.has(rest)) continue;
        const name = colourName(prefix, rest);
        if (!name || /^\d/.test(name)) continue;
        if (NON_COLOR.has(name)) continue;
        if (definedColors.has(name)) continue;
        unknown.push(`${file.path}: ${token}`);
      }
    }
    expect(unknown).toEqual([]);
  });

  it("never uses the bare `rounded` utility", () => {
    const offenders: string[] = [];
    for (const file of files) {
      for (const token of file.tokens) {
        if (token === "rounded") offenders.push(file.path);
      }
    }
    expect(offenders).toEqual([]);
  });

  it("never divides with a translucent border", () => {
    const offenders: string[] = [];
    for (const file of files) {
      for (const token of file.tokens) {
        if (/^border-line(-strong)?\/\d+$/.test(token)) {
          offenders.push(`${file.path}: ${token}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it("never sets text below the 3xs floor", () => {
    const offenders: string[] = [];
    for (const file of files) {
      for (const token of file.tokens) {
        const match = token.match(/^text-\[(\d+(?:\.\d+)?)px\]$/);
        if (match && Number(match[1]) < 10) {
          offenders.push(`${file.path}: ${token}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
