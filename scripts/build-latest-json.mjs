#!/usr/bin/env node
/**
 * Build the Tauri updater manifest (`latest.json`) from staged release assets.
 *
 * Tauri's `tauri-action` normally does this, but this repository builds with a
 * plain `pnpm tauri build` (a matrix with a separate checksum job), so the
 * manifest is generated here instead. The format is documented at
 * https://v2.tauri.app/plugin/updater/ (Static JSON File).
 *
 * Fail closed: every platform entry is required. A release that silently lacks
 * one platform's payload would leave those users unable to update, and Tauri
 * validates the whole manifest before reading the version, so a partial file
 * would also break the platforms that *are* present.
 *
 * Usage:
 *   node scripts/build-latest-json.mjs \
 *     --artifacts artifacts \
 *     --tag v0.6.0 \
 *     --repo taitai2661/MIX-agent \
 *     --out artifacts/latest.json
 */
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function arg(name, fallback = null) {
  const index = process.argv.indexOf(`--${name}`);
  if (index === -1) return fallback;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`--${name} needs a value`);
  }
  return value;
}

const artifactsDir = resolve(arg("artifacts", "artifacts"));
const tag = arg("tag");
const repo = arg("repo", "taitai2661/MIX-agent");
const outPath = resolve(arg("out", join(artifactsDir, "latest.json")));

if (!tag) {
  throw new Error("--tag is required (the Release tag the assets are uploaded to)");
}

/** Every updater payload and the platform key it satisfies. */
const PLATFORMS = [
  {
    key: "darwin-aarch64",
    asset: "MIX-agent-Desktop-Apple-Silicon.app.tar.gz",
  },
  {
    key: "darwin-x86_64",
    asset: "MIX-agent-Desktop-Intel.app.tar.gz",
  },
  {
    key: "windows-x86_64",
    asset: "MIX-agent-Desktop-Windows-x64-Setup.exe",
  },
  {
    key: "linux-x86_64",
    asset: "MIX-agent-Desktop-Linux-x64.AppImage",
  },
];

/** Recursively index files by basename under `dir`. */
function indexFiles(dir) {
  const found = new Map();
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      for (const [name, nested] of indexFiles(path)) {
        found.set(name, nested);
      }
    } else {
      found.set(entry, path);
    }
  }
  return found;
}

const files = indexFiles(artifactsDir);
const config = JSON.parse(
  readFileSync(join(root, "apps/desktop/app/tauri.conf.json"), "utf8"),
);
const version = config.version;

if (!version) {
  throw new Error("apps/desktop/app/tauri.conf.json has no version");
}

const platforms = {};
for (const { key, asset } of PLATFORMS) {
  const payload = files.get(asset);
  if (!payload) {
    throw new Error(`missing updater payload for ${key}: ${asset}`);
  }
  const signaturePath = `${payload}.sig`;
  const signature = readFileSync(signaturePath, "utf8").trim();
  if (!signature) {
    throw new Error(`empty signature for ${asset}`);
  }
  platforms[key] = {
    signature,
    url: `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(asset)}`,
  };
}

const manifest = {
  version,
  notes: process.env.MIX_RELEASE_NOTES ?? "",
  pub_date: new Date().toISOString(),
  platforms,
};

writeFileSync(outPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`latest.json -> ${outPath} (version ${version}, tag ${tag})`);
console.log(JSON.stringify(manifest.platforms, null, 2));
