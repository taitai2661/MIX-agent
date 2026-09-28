#!/usr/bin/env node
/**
 * Download a Chromium build into app/resources/chromium so the packaged app
 * ships its own browser instead of relying on whatever the user has installed.
 *
 * Layout matches `mixd_browser::profile::find_bundled_chromium`:
 *   macOS    chromium/chrome-mac/Chromium.app/Contents/MacOS/Chromium
 *   Windows  chromium/chrome-win/chrome.exe
 *   Linux    chromium/chrome-linux/chrome
 *
 * Source: the official Chrome for Testing endpoints, so no extra npm
 * dependency and no guessing at CDN paths.
 *
 * Usage:  node scripts/fetch-chromium.mjs [--force] [--version <x>]
 * Env:    MIX_SKIP_CHROMIUM=1 skips the download entirely.
 *         MIX_CHROMIUM_CHANNEL=beta|canary|dev picks a non-stable channel.
 */
import { existsSync } from "node:fs";
import { chmod, mkdir, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import os from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const outRoot = resolve(here, "..", "app", "resources", "chromium");
const args = process.argv.slice(2);
const force = args.includes("--force");
const pinned = args.includes("--version")
  ? args[args.indexOf("--version") + 1]
  : process.env.MIX_CHROMIUM_VERSION ?? null;

/** @typedef {"mac"|"win"|"linux"} Platform */

function detectPlatform() {
  if (os.platform() === "darwin") return "mac";
  if (os.platform() === "win32") return "win";
  return "linux";
}

/** Chrome for Testing platform identifier, which folds in the CPU arch. */
function cftPlatform(platform) {
  const arm = os.arch() === "arm64";
  if (platform === "mac") return arm ? "mac-arm64" : "mac-x64";
  if (platform === "win") return arm ? "win64" : "win32";
  return "linux64";
}

/** Final on-disk path we promise to the Rust side. */
function targetBinary(platform) {
  if (platform === "mac") {
    return join(outRoot, "chrome-mac", "Chromium.app", "Contents", "MacOS", "Chromium");
  }
  if (platform === "win") return join(outRoot, "chrome-win", "chrome.exe");
  return join(outRoot, "chrome-linux", "chrome");
}

/** Directory name inside the downloaded archive. */
function archiveRoot(cft) {
  // Chrome for Testing ships chrome-mac-arm64 / chrome-win64 / chrome-linux64.
  return cft.startsWith("mac") ? `chrome-${cft}` : cft.replace("win32", "win32");
}

async function exists(path) {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

async function resolveVersion(platform) {
  if (pinned) return pinned;
  const channel = process.env.MIX_CHROMIUM_CHANNEL ?? "Stable";
  const url = `https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json`;
  console.log(`[chromium] resolving ${channel} from chrome-for-testing…`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`version lookup failed: ${res.status} ${res.statusText}`);
  const json = await res.json();
  const entry = json.channels?.[channel];
  if (!entry?.version) throw new Error(`no "${channel}" channel in chrome-for-testing response`);
  return entry.version;
}

async function download(url, dest) {
  console.log(`[chromium] GET ${url}`);
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`download failed: ${res.status} ${res.statusText}`);
  const buf = Buffer.from(await res.arrayBuffer());
  await writeFile(dest, buf);
  return buf.length;
}

async function unpack(archive, dest) {
  await mkdir(dest, { recursive: true });
  if (os.platform() === "darwin") {
    execFileSync("unzip", ["-q", "-o", archive, "-d", dest], { stdio: "inherit" });
  } else if (os.platform() === "win32") {
    execFileSync(
      "powershell",
      ["-NoProfile", "-Command",
        `Expand-Archive -Force -LiteralPath '${archive}' -DestinationPath '${dest}'`],
      { stdio: "inherit" },
    );
  } else {
    execFileSync("unzip", ["-q", "-o", archive, "-d", dest], { stdio: "inherit" });
  }
}

async function main() {
  if (process.env.MIX_SKIP_CHROMIUM === "1") {
    console.log("[chromium] MIX_SKIP_CHROMIUM=1 — skipping (app falls back to system Chromium)");
    return;
  }
  const platform = detectPlatform();
  const cft = cftPlatform(platform);
  const dest = targetBinary(platform);

  if (!force && (await exists(dest))) {
    console.log(`[chromium] already present: ${dest}`);
    return;
  }

  const version = await resolveVersion(platform);
  const cdl = `https://storage.googleapis.com/chrome-for-testing-public/${version}/${cft}/chrome-${cft}.zip`;
  const tmp = join(os.tmpdir(), `mix-chromium-${version}-${cft}.zip`);

  const bytes = await download(cdl, tmp);
  console.log(`[chromium] ${(bytes / 1e6).toFixed(1)} MB, extracting…`);

  await rm(outRoot, { recursive: true, force: true });
  await mkdir(outRoot, { recursive: true });
  await unpack(tmp, outRoot);
  await rm(tmp, { force: true });

  // Normalise the archive's own directory name to the layout the Rust resolver
  // looks for, so we do not have to duplicate Chrome for Testing's naming.
  const extracted = join(outRoot, archiveRoot(cft));
  const normalized =
    platform === "mac" ? join(outRoot, "chrome-mac")
      : platform === "win" ? join(outRoot, "chrome-win")
        : join(outRoot, "chrome-linux");
  await mkdir(normalized, { recursive: true });
  if (platform === "mac") {
    // The bundle is named "Google Chrome for Testing.app" upstream; do not
    // hardcode it, just take the single .app we just unpacked.
    const bundles = (await readdir(extracted, { withFileTypes: true }))
      .filter((e) => e.isDirectory() && e.name.endsWith(".app"))
      .map((e) => e.name);
    if (bundles.length !== 1) {
      throw new Error(
        `expected exactly one .app bundle in ${extracted}, found: ${bundles.join(", ") || "(none)"}`,
      );
    }
    await rename(join(extracted, bundles[0]), join(normalized, "Chromium.app"));
    await rm(extracted, { recursive: true, force: true });

    // Upstream also names the inner executable after the product, so rename it
    // to the single stable name the Rust resolver and the CDP flags expect.
    const macosDir = join(normalized, "Chromium.app", "Contents", "MacOS");
    const inners = await readdir(macosDir, { withFileTypes: true });
    const inner = inners.find((e) => e.isFile() && e.name !== "Chromium");
    if (!inner) throw new Error(`no executable found in ${macosDir}`);
    await rename(join(macosDir, inner.name), join(macosDir, "Chromium"));
  } else {
    await rename(extracted, normalized);
  }

  if (platform !== "win") await chmod(dest, 0o755);

  if (!existsSync(dest)) {
    const listing = await readdir(outRoot).catch(() => []);
    throw new Error(`extraction did not produce ${dest}\nfound: ${listing.join(", ")}`);
  }
  console.log(`[chromium] ready: ${dest}`);
}

main().catch((err) => {
  console.error(`[chromium] ${err.message}`);
  console.error("[chromium] the app falls back to a system Chromium when this is absent.");
  process.exit(1);
});
