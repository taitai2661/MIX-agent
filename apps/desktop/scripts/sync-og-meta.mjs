// OGP 画像のキャッシュバスティングを保証する prebuild スクリプト。
//
// `ui/public/og-image.png` の sha256（先頭 8 桁）を `ui/index.html` 内の
// `og:image` / `twitter:image` の `?v=<hash>` に付け替える。画像を更新して
// `pnpm build` するだけで URL が変わるため、X / Slack / LINE などの
// クローラキャッシュに古い画像が残らない。
//
// 使い方: `pnpm --filter mix-agent-desktop-ui build` が prebuild 経由で自動実行。
// 単体実行: `node scripts/sync-og-meta.mjs`（cwd は mix-agent-desktop/ 想定だが
// スクリプト自身の位置から解決するのでどこから呼んでもよい）。

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const uiDir = join(here, "..", "ui");
const imagePath = join(uiDir, "public", "og-image.png");
const htmlPath = join(uiDir, "index.html");

if (!existsSync(imagePath)) {
  console.error(`[sync-og-meta] missing OGP image: ${imagePath}`);
  process.exit(1);
}

const digest = createHash("sha256")
  .update(readFileSync(imagePath))
  .digest("hex")
  .slice(0, 8);

let html = readFileSync(htmlPath, "utf8");
const before = html;
// バージョン付き (`?v=<hex>`) も素の参照も正規化する。
html = html.replaceAll(/\/og-image\.png\?v=[0-9a-f]*/g, `/og-image.png?v=${digest}`);
if (html.includes('content="/og-image.png"')) {
  html = html.replaceAll('content="/og-image.png"', `content="/og-image.png?v=${digest}"`);
}

if (html !== before) {
  writeFileSync(htmlPath, html);
  console.log(`[sync-og-meta] og:image version -> v=${digest}`);
} else {
  console.log(`[sync-og-meta] og:image already at v=${digest}`);
}
