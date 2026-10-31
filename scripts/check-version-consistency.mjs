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

function readProjectVersion(relativePath, section = "[project]") {
  const text = readFileSync(join(root, relativePath), "utf8");
  const sectionStart = text.indexOf(section);
  if (sectionStart === -1) {
    throw new Error(`${relativePath} has no ${section} section`);
  }
  const body = text.slice(sectionStart + section.length);
  const nextSection = body.search(/\n\s*\[/);
  const scope = nextSection === -1 ? body : body.slice(0, nextSection);
  const match = scope.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`${relativePath} has no version in ${section}`);
  }
  return match[1];
}

function readRegexVersion(relativePath, pattern, label) {
  const text = readFileSync(join(root, relativePath), "utf8");
  const match = text.match(pattern);
  if (!match) {
    throw new Error(`${relativePath} has no ${label}`);
  }
  return match[1];
}

/**
 * The ZimaOS store Compose is generated from `compose.yaml`, so it carries two
 * version surfaces the manifests above do not: the GHCR image tags and the
 * dashboard metadata `version`. Both drifted silently before (v0.5.2 assets
 * next to a v0.5.4 app), so they are checked here rather than trusted.
 */
function readStoreCompose(relativePath) {
  const text = readFileSync(join(root, relativePath), "utf8");
  const images = [...text.matchAll(/image:\s*ghcr\.io\/[^:\s]+-[a-z-]+:v(\d+\.\d+\.\d+)/g)].map(
    (match) => match[1],
  );
  const metadata = text.match(/^\s*version:\s*(\d+\.\d+\.\d+)\s*$/m);
  if (images.length === 0) {
    throw new Error(`${relativePath} has no GHCR image tags`);
  }
  if (!metadata) {
    throw new Error(`${relativePath} has no store metadata version`);
  }
  return { images, metadata: metadata[1] };
}

const storeCompose = readStoreCompose("apps/MIX-agent/docker-compose.yml");
const storeMetaKeys = [
  ...readFileSync(join(root, "scripts/render_zimaos_compose.py"), "utf8").matchAll(
    /^\s{4}"v(\d+\.\d+\.\d+)":\s*\{/gm,
  ),
].map((match) => match[1]);

const expected = readJson("package.json").version;

const versions = [
  ["package.json", expected],
  ["apps/web/package.json", readJson("apps/web/package.json").version],
  ["apps/desktop/package.json", readJson("apps/desktop/package.json").version],
  ["apps/desktop/ui/package.json", readJson("apps/desktop/ui/package.json").version],
  ["apps/desktop/app/tauri.conf.json", readJson("apps/desktop/app/tauri.conf.json").version],
  [
    "apps/desktop/Cargo.toml",
    readProjectVersion("apps/desktop/Cargo.toml", "[workspace.package]"),
  ],
  ["apps/server/pyproject.toml", readProjectVersion("apps/server/pyproject.toml")],
  [
    "apps/server/mix_agent/main.py",
    readRegexVersion(
      "apps/server/mix_agent/main.py",
      /FastAPI\([^)]*version="([^"]+)"/,
      "FastAPI version",
    ),
  ],
  ["docs/openapi.json", readJson("docs/openapi.json").info.version],
  [
    "runners/mcp_manager/main.py",
    readRegexVersion(
      "runners/mcp_manager/main.py",
      /"io\.modelcontextprotocol\/clientInfo":\s*\{\s*"name":\s*"MIX agent",\s*"version":\s*"([^"]+)"/,
      "MCP clientInfo version",
    ),
  ],
  [
    "apps/server/mix_agent/mcp/protocol.py",
    readRegexVersion(
      "apps/server/mix_agent/mcp/protocol.py",
      /"io\.modelcontextprotocol\/clientInfo"\]?\s*=\s*\{\s*"name":\s*"MIX agent",\s*"version":\s*"([^"]+)"/,
      "MCP clientInfo version",
    ),
  ],
  ["apps/MIX-agent/docker-compose.yml", storeCompose.metadata],
  ...storeCompose.images.map((version, index) => [
    `apps/MIX-agent/docker-compose.yml (image ${index + 1})`,
    version,
  ]),
  [
    "scripts/render_zimaos_compose.py (RELEASE_META)",
    storeMetaKeys.includes(expected) ? expected : `missing v${expected} entry [${storeMetaKeys.join(", ")}]`,
  ],
];

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
