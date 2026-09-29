/// Static coverage test for the `en` dictionary in `lib/i18n.ts`.
///
/// It reads the sources from disk (Node `fs` + a recursive walk — `vitest`'s
/// `import.meta.glob` is not available the way this suite is configured) and
/// checks two things:
///
///  1. Every *literal* argument of `t("…")` / `tf("…")` in `src/**` exists in
///     the `en` dictionary. A missing entry means the English UI silently
///     falls back to the Japanese original.
///  2. Dictionary keys that no source references at all ("orphans") are
///     compared against an allowlist, so the set may shrink but never grow.
///
/// Exclusions (by relative path from `src/`):
///  - `lib/i18n.ts` itself — it *is* the dictionary.
///  - any `*.test.ts` / `*.test.tsx` — tests legitimately translate keys that
///    no production code uses (and would otherwise force fake entries).
///
/// Everything else is scanned, including files other tasks own (App.tsx,
/// ChatPane.tsx, …), because a key they reference still has to be registered.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/** Resolves `ui/src` without relying on `import.meta.url` (vitest serves it
 *  as a non-`file:` URL) — walk up from the cwd looking for the marker file. */
function resolveSrcRoot(): string {
  const candidates: string[] = [];
  try {
    const here = path.dirname(fileURLToPath(import.meta.url));
    candidates.push(here, path.dirname(here));
  } catch {
    // Not a file: URL in this environment; fall back to the cwd walk below.
  }
  candidates.push(path.resolve(process.cwd(), "src"));
  let dir = process.cwd();
  for (let i = 0; i < 6; i += 1) {
    candidates.push(path.join(dir, "src"));
    dir = path.dirname(dir);
  }
  for (const candidate of candidates) {
    if (fs.existsSync(path.join(candidate, "lib", "i18n.ts"))) return candidate;
  }
  throw new Error(`could not locate ui/src from ${process.cwd()}`);
}

const SRC_ROOT = resolveSrcRoot();

/** Dictionary keys that are intentionally unreferenced today. */
const ORPHAN_ALLOWLIST: readonly string[] = [
  // Reserved for surfaces that still render them raw (ChatPane / Transcript /
  // ActivityPanel / Composer attach-limit copy are owned by other tasks).
  "添付ファイルが大きすぎます (上限8MB)",
  "画像は送信されません",
  "Enter で送信 · Shift+Enter で改行",
  "深い思考",
  "まだメッセージがありません",
  "会話を選択するか、新規スレッドを作成してください",
  "実行中…",
  "ツール実行",
  "コピーしました",
  "この会話をMarkdownで書き出す",
  "行追加",
  "行削除",
  "もっと見る",
  // Spare / not yet wired to any pane.
  "ステップ",
  "詳細",
  "無効",
  "はい",
  "いいえ",
  "削除しました",
  "隠しファイル",
];

function listSources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...listSources(full));
    else if (/\.tsx?$/.test(entry.name)) out.push(full);
  }
  return out;
}

function isExcluded(absPath: string): boolean {
  const rel = path.relative(SRC_ROOT, absPath).split(path.sep).join("/");
  if (rel === "lib/i18n.ts") return true;
  return /\.test\.tsx?$/.test(rel);
}

const unescape = (raw: string) => raw.replace(/\\(.)/g, "$1");

/** Reads the flat `const en: Record<string, string> = { … }` object literal. */
function parseDictionary(source: string): Map<string, string> {
  const marker = "const en: Record<string, string> = {";
  const at = source.indexOf(marker);
  if (at < 0) throw new Error("could not find the `en` dictionary in i18n.ts");

  const entries = new Map<string, string>();
  let i = at + marker.length - 1; // position of the opening brace
  let depth = 0;

  const skipTrivia = () => {
    for (;;) {
      while (i < source.length && /\s/.test(source[i])) i += 1;
      if (source.startsWith("//", i)) {
        while (i < source.length && source[i] !== "\n") i += 1;
        continue;
      }
      if (source.startsWith("/*", i)) {
        i += 2;
        while (i < source.length && !source.startsWith("*/", i)) i += 1;
        i += 2;
        continue;
      }
      return;
    }
  };

  const readString = (): string | null => {
    const quote = source[i];
    if (quote !== '"' && quote !== "'") return null;
    i += 1;
    let out = "";
    while (i < source.length) {
      const ch = source[i];
      if (ch === "\\") {
        out += source[i + 1] ?? "";
        i += 2;
        continue;
      }
      if (ch === quote) {
        i += 1;
        return out;
      }
      out += ch;
      i += 1;
    }
    return null;
  };

  for (; i < source.length; i += 1) {
    const ch = source[i];
    if (source.startsWith("//", i)) {
      while (i < source.length && source[i] !== "\n") i += 1;
      continue;
    }
    if (source.startsWith("/*", i)) {
      i += 2;
      while (i < source.length && !source.startsWith("*/", i)) i += 1;
      i += 2;
      continue;
    }
    if (ch === "{") {
      depth += 1;
      continue;
    }
    if (ch === "}") {
      depth -= 1;
      if (depth === 0) break;
      continue;
    }
    if (depth === 1 && (ch === '"' || ch === "'")) {
      const key = readString();
      i -= 1; // the for-loop increments
      if (key === null) continue;
      i += 1;
      skipTrivia();
      if (source[i] !== ":") continue;
      i += 1;
      skipTrivia();
      const value = readString();
      entries.set(key, value ?? "");
      i -= 1; // the for-loop increments past the closing quote
    }
  }
  return entries;
}

const T_CALL = /\b(?:t|tf)\(\s*(["'])((?:\\.|(?!\1).)*)\1/g;
const ANY_STRING = /(["'])((?:\\.|(?!\1).)*)\1/g;

describe("i18n key coverage", () => {
  const sources = listSources(SRC_ROOT).filter((f) => !isExcluded(f));

  it("scans the sources with fs (no import.meta.glob)", () => {
    expect(sources.length).toBeGreaterThan(30);
    expect(sources.some((f) => f.endsWith(path.join("lib", "i18n.ts")))).toBe(false);
    expect(sources.some((f) => f.endsWith(".test.tsx"))).toBe(false);
  });

  it("registers every literal t()/tf() key in the en dictionary", () => {
    const dict = parseDictionary(fs.readFileSync(path.join(SRC_ROOT, "lib", "i18n.ts"), "utf8"));
    const missing: string[] = [];

    for (const file of sources) {
      const code = fs.readFileSync(file, "utf8");
      let match: RegExpExecArray | null;
      T_CALL.lastIndex = 0;
      while ((match = T_CALL.exec(code)) !== null) {
        const key = unescape(match[2]);
        if (!dict.has(key)) {
          const line = code.slice(0, match.index).split("\n").length;
          const rel = path.relative(SRC_ROOT, file).split(path.sep).join("/");
          missing.push(`${rel}:${line}  ${JSON.stringify(key)}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("keeps orphan dictionary keys from growing", () => {
    const dict = parseDictionary(fs.readFileSync(path.join(SRC_ROOT, "lib", "i18n.ts"), "utf8"));

    // A key counts as referenced when its literal appears anywhere in the
    // sources: either as a `t("…")` argument, or inside a constant table that
    // a component passes to `t()` later (e.g. `ALL_MODES`, `SETTINGS_TARGETS`).
    const referenced = new Set<string>();
    for (const file of sources) {
      const code = fs.readFileSync(file, "utf8");
      let match: RegExpExecArray | null;
      ANY_STRING.lastIndex = 0;
      while ((match = ANY_STRING.exec(code)) !== null) referenced.add(unescape(match[2]));
    }

    const allow = new Set(ORPHAN_ALLOWLIST);
    const unexpected = [...dict.keys()].filter((k) => !referenced.has(k) && !allow.has(k)).sort();
    expect(unexpected).toEqual([]);
  });

  it("does not register the same key twice", () => {
    const source = fs.readFileSync(path.join(SRC_ROOT, "lib", "i18n.ts"), "utf8");
    const seen: string[] = [];
    const duplicates: string[] = [];
    // Re-walk the object literal collecting every key occurrence.
    const marker = "const en: Record<string, string> = {";
    const body = source.slice(source.indexOf(marker));
    const keyAt = /^\s{2}"((?:\\.|[^"\\])*)":/gm;
    let match: RegExpExecArray | null;
    while ((match = keyAt.exec(body)) !== null) {
      const key = unescape(match[1]);
      if (seen.includes(key)) duplicates.push(key);
      seen.push(key);
    }
    expect(duplicates).toEqual([]);
  });
});
