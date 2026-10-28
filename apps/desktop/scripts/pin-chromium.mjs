#!/usr/bin/env node
/**
 * Pin the Chrome for Testing build that the desktop app downloads at runtime.
 *
 * Usage:
 *   node scripts/pin-chromium.mjs [--version <x>] [--channel Stable] [--out <path>]
 *
 * The generated manifest is compiled into `mixd-browser`
 * (`browser/src/chromium-manifest.json`). Release CI verifies that the pinned
 * URLs still resolve and that the downloaded zip hashes match; it no longer
 * downloads Chromium into the app bundle.
 *
 * Chrome for Testing publishes no detached signatures, so SHA-256 is the
 * supply-chain anchor. Review the diff and commit it like a dependency bump.
 */
import { createHash } from "node:crypto";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import os from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const defaultOut = resolve(here, "..", "browser", "src", "chromium-manifest.json");
const args = process.argv.slice(2);
const argValue = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : null;
};

const channel = argValue("--channel") ?? "Stable";
const pinned = argValue("--version");
const out = resolve(argValue("--out") ?? defaultOut);
const verifyOnly = args.includes("--verify");
const cache = resolve(argValue("--cache") ?? join(os.tmpdir(), "mix-agent-cft"));

/** Platforms the desktop release actually supports. */
const PLATFORMS = ["mac-arm64", "mac-x64", "win64", "linux64"];

async function resolveVersion() {
  if (pinned) return pinned;
  const url =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
  console.log(`[chromium-pin] resolving ${channel}…`);
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`version lookup failed: ${response.status} ${response.statusText}`);
  }
  const json = await response.json();
  const version = json.channels?.[channel]?.version;
  if (!version) throw new Error(`no "${channel}" channel in the response`);
  return version;
}

async function download(platform, version) {
  const url = `https://storage.googleapis.com/chrome-for-testing-public/${version}/${platform}/chrome-${platform}.zip`;
  await mkdir(cache, { recursive: true });
  const path = join(cache, `chrome-${platform}-${version}.zip`);
  // Also accept the unversioned name used by ad-hoc downloads, so a local
  // cache can be reused while developing this script.
  for (const candidate of [path, join(cache, `chrome-${platform}.zip`)]) {
    try {
      const info = await stat(candidate);
      if (info.size > 0) {
        console.log(`[chromium-pin] cached ${platform} (${(info.size / 1e6).toFixed(1)} MB)`);
        return { path: candidate, size: info.size, url };
      }
    } catch {
      // try the next name
    }
  }
  console.log(`[chromium-pin] downloading ${platform}…`);
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`${platform}: download failed (${response.status} ${response.statusText})`);
  }
  const finalUrl = new URL(response.url);
  if (finalUrl.protocol !== "https:" || finalUrl.hostname !== "storage.googleapis.com") {
    throw new Error(`${platform}: unexpected download host ${finalUrl.origin}`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  await writeFile(path, bytes);
  return { path, size: bytes.length, url };
}

async function sha256(path) {
  return createHash("sha256").update(await readFile(path)).digest("hex");
}

async function verifyManifest() {
  const manifest = JSON.parse(await readFile(out, "utf8"));
  let failed = false;
  for (const [platform, expected] of Object.entries(manifest.platforms)) {
    const artifact = await download(platform, manifest.version);
    const got = await sha256(artifact.path);
    if (artifact.url !== expected.url || got !== expected.sha256 || artifact.size !== expected.size) {
      console.error(
        `[chromium-pin] MISMATCH ${platform}: url=${artifact.url} size=${artifact.size} sha256=${got}`,
      );
      failed = true;
    } else {
      console.log(`[chromium-pin] verified ${platform} ${manifest.version}`);
    }
  }
  if (failed) throw new Error("the pinned Chromium manifest does not match the upstream archives");
  console.log(`[chromium-pin] ${Object.keys(manifest.platforms).length} platforms verified`);
}

async function main() {
  if (verifyOnly) {
    await verifyManifest();
    return;
  }
  const version = await resolveVersion();
  const platforms = {};
  for (const platform of PLATFORMS) {
    const artifact = await download(platform, version);
    platforms[platform] = {
      url: artifact.url,
      sha256: await sha256(artifact.path),
      size: artifact.size,
    };
  }
  const manifest = { version, channel, platforms };
  await writeFile(out, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`[chromium-pin] wrote ${out}`);
  console.log(`[chromium-pin] version ${version}`);
}

main().catch((error) => {
  console.error(`[chromium-pin] ${error.message}`);
  process.exit(1);
});
