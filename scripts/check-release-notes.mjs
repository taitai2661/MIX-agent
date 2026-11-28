#!/usr/bin/env node
/**
 * Refuse to publish a release whose notes are missing or explicitly a draft.
 *
 * Usage:
 *   node scripts/check-release-notes.mjs --tag v0.5.4
 *   node scripts/check-release-notes.mjs --tag desktop-v0.5.4
 *
 * A non-release ref (branch name in a dry-run) is ignored.
 */
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const tagIndex = process.argv.indexOf("--tag");
const rawTag = (tagIndex === -1 ? "" : (process.argv[tagIndex + 1] ?? "")).trim();
const version = rawTag.replace(/^desktop-/, "").replace(/^v/, "");

if (!/^\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.-]+)?$/.test(version)) {
  console.log(`tag ${rawTag || "(none)"} is not a release tag; skipping release-note check`);
  process.exit(0);
}

const file = join(root, "docs", `release-notes-v${version}.md`);
if (!existsSync(file)) {
  console.error(`::error::docs/release-notes-v${version}.md is missing; write the release notes before tagging.`);
  process.exit(1);
}

const text = readFileSync(file, "utf8");
// Only the explicit draft phrases in the header are a release blocker. A bare
// 未公開 appears in ordinary release notes (e.g. 「未公開メトリクス」 or 「非公開
// バックエンド」), so matching it anywhere rejected perfectly publishable notes.
// See docs/release-notes-*.md: a file marked as an unpublished draft blocks release.
const draftMarkers = [
  /\barchived draft\b/i,
  /\bwas not published\b/i,
  /\bunpublished draft\b/i,
  /未公開のドラフト/,
];
const headerLines = text.split(/\r?\n/).slice(0, 12);
for (const marker of draftMarkers) {
  if (headerLines.some((line) => marker.test(line))) {
    console.error(
      `::error::docs/release-notes-v${version}.md is marked as an unpublished draft (${marker}); ` +
        "publish real notes or choose the next version.",
    );
    process.exit(1);
  }
}

if (text.trim().length < 200) {
  console.error(`::error::docs/release-notes-v${version}.md is too short to be a release body.`);
  process.exit(1);
}

console.log(`Release notes OK: docs/release-notes-v${version}.md`);
