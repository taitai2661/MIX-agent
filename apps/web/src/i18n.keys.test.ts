/// Static coverage test for the English dictionary in `app/i18n.ts`.
///
/// The web UI translates by passing the *Japanese* string to `t()`, so a
/// missing `en` entry silently falls back to Japanese instead of failing. This
/// scans every source module and checks that each literal `t("…")` argument has
/// an entry, mirroring `apps/desktop/ui/src/lib/i18n.keys.test.ts`.
///
/// Exclusions:
///  - `app/i18n.ts` — the dictionary and lookup itself.
///  - `app/strings.ts` — legacy constants, translated where they are used.
///  - `*.test.ts(x)` — tests may reference keys on purpose.
///  - `generated/` — the OpenAPI client.

import { describe, expect, it } from "vitest";
import { en } from "./app/i18n";

const SOURCES = import.meta.glob("./**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const EXCLUDED = new Set(["./app/i18n.ts", "./app/strings.ts"]);

function included(path: string): boolean {
  return (
    !EXCLUDED.has(path) &&
    !/\.test\.tsx?$/.test(path) &&
    !path.startsWith("./generated/")
  );
}

function literalKeys(source: string): string[] {
  const keys: string[] = [];
  for (const match of source.matchAll(/\bt\(\s*("(?:[^"\\]|\\.)*")\s*\)/g)) {
    keys.push(JSON.parse(match[1]) as string);
  }
  return keys;
}

describe("English dictionary coverage", () => {
  it("has an entry for every literal t() key used by the UI", () => {
    const missing: string[] = [];
    for (const [path, source] of Object.entries(SOURCES)) {
      if (!included(path)) continue;
      for (const key of literalKeys(source)) {
        if (!(key in en)) missing.push(`${path}: ${JSON.stringify(key)}`);
      }
    }
    expect(missing).toEqual([]);
  });

  it("keeps dynamic keys out of the literal scan", () => {
    // A template-literal key cannot be verified statically; make the
    // convention explicit so a future t(`…`) shows up in review.
    const offenders = Object.entries(SOURCES)
      .filter(([path, source]) => included(path) && /\bt\(\s*`/.test(source))
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });
});
