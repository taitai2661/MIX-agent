#!/usr/bin/env node
/**
 * Keep every user-visible version in lockstep.
 *
 * The release tag, the monorepo package, the desktop workspace/UI packages,
 * the Tauri bundle config and the Cargo workspace each carry a version. A
 * mismatch produces installers whose filename, About box and MSI metadata
 * disagree with the tag, which is hard to spot after publication.
 *
 * Usage:
 *   node scripts/check-version-consistency.mjs
 *   node scripts/check-version-consistency.mjs --tag v0.6.0
 *   node scripts/check-version-consistency.mjs --tag desktop-v0.6.0
 *
 * A non-release ref (for example `main` in a release dry-run) is accepted and
 * only the manifest-to-manifest comparison runs.
 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const tagFromArgs = (() => {
  const index = process.argv.indexOf("--tag");
  return index === -1 ? "" : (process.argv[index + 1] ?? "");
})();

function readJson(relativePath) {
  return JSON.parse(readFileSync(join(root, relativePath), "utf8"));
}

function readCargoWorkspaceVersion(relativePath) {
  const text = readFileSync(join(root, relativePath), "utf8");
  const sectionStart = text.indexOf("[workspace.package]");
  if (sectionStart === -1) {
    throw new Error(`${relativePath} has no [workspace.package] section`);
  }
  const section = text.slice(sectionStart);
  const nextSection = section.indexOf("\n[", 1);
  const body = nextSection === -1 ? section : section.slice(0, nextSection);
  const match = body.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`${relativePath} has no workspace.package version`);
  }
  return match[1];
}

const versions = [
  ["package.json", readJson("package.json").version],
  ["apps/desktop/package.json", readJson("apps/desktop/package.json").version],
  ["apps/desktop/ui/package.json", readJson("apps/desktop/ui/package.json").version],
  ["apps/desktop/app/tauri.conf.json", readJson("apps/desktop/app/tauri.conf.json").version],
  ["apps/desktop/Cargo.toml", readCargoWorkspaceVersion("apps/desktop/Cargo.toml")],
];

const expected = versions[0][1];
let failed = false;

for (const [file, version] of versions) {
  const status = version === expected ? "ok" : "MISMATCH";
  console.log(`${status.padEnd(8)} ${file}: ${version}`);
  if (version !== expected) failed = true;
}

if (failed) {
  console.error(`\nVersion mismatch: expected every manifest to be ${expected}.`);
  process.exit(1);
}

// Tags may be `v0.6.0` or `desktop-v0.6.0`. Branch names and other refs are
// intentionally ignored so `workflow_dispatch` dry-runs can pass `tag=main`.
const tagVersion = tagFromArgs
  .replace(/^desktop-v/, "")
  .replace(/^v/, "")
  .trim();

if (tagVersion && /^\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.-]+)?$/.test(tagVersion)) {
  console.log(`tag     ${tagFromArgs}: ${tagVersion}`);
  if (tagVersion !== expected) {
    console.error(`\nRelease tag ${tagFromArgs} does not match app version ${expected}.`);
    process.exit(1);
  }
} else if (tagFromArgs) {
  console.log(`tag     ${tagFromArgs}: not a release tag; skipping tag comparison`);
}

console.log(`\nVersion consistency OK: ${expected}`);
