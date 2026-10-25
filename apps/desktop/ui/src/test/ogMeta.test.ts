import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// OGP メタの回帰テスト。画像を差し替えて `pnpm build` すれば
// prebuild の sync-og-meta.mjs が `?v=` を付け替える前提で、
// index.html の参照と実ファイルのハッシュ一致まで検証する
// （X / Slack / LINE のキャッシュに古い画像が残らないことの担保）。
// vitest の cwd は ui パッケージ直下のため相対解決で十分。
const htmlPath = join(process.cwd(), "index.html");
const imagePath = join(process.cwd(), "public", "og-image.png");

describe("OGP meta", () => {
  it("has the required og:/twitter: tags with a versioned image", () => {
    const html = readFileSync(htmlPath, "utf8");
    for (const tag of [
      'property="og:title"',
      'property="og:description"',
      'property="og:image"',
      'property="og:image:width" content="1200"',
      'property="og:image:height" content="630"',
      'name="twitter:card" content="summary_large_image"',
      'name="twitter:image"',
    ]) {
      expect(html, tag).toContain(tag);
    }
    expect(html).toMatch(/\/og-image\.png\?v=[0-9a-f]{8}/);
  });

  it("points at the current og-image.png hash and the file exists", () => {
    expect(existsSync(imagePath), "public/og-image.png").toBe(true);
    const digest = createHash("sha256")
      .update(readFileSync(imagePath))
      .digest("hex")
      .slice(0, 8);
    const html = readFileSync(htmlPath, "utf8");
    expect(html).toContain(`/og-image.png?v=${digest}`);
    // og:image と twitter:image の両方が同じバージョンを指すこと。
    const versions = new Set(
      [...html.matchAll(/\/og-image\.png\?v=([0-9a-f]{8})/g)].map((m) => m[1]),
    );
    expect([...versions]).toEqual([digest]);
  });
});
