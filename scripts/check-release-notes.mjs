#!/usr/bin/env node
/**
 * Refuse to publish a release whose notes are missing or explicitly a draft.
 *
 * `docs/README.md` records that `release-notes-v0.4.0.md` is an unpublished
 * draft, but nothing stopped a `v0.4.0` tag from publishing it. The desktop
 * workflow also used to fall back to a "notes coming soon" body, which made a
 * missing file look like a successful release.
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
const draftMarkers = [
  /archived draft/i,
  /was not published/i,
  /unpublished draft/i,
  /未公開のドラフト/,
  /未公開/,
];
for (const marker of draftMarkers) {
  if (marker.test(text)) {
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
