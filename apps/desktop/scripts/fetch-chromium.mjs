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
 * Usage:  node scripts/fetch-chromium.mjs [--force] [--version <x>] [--cft <platform>]
 * Env:    MIX_SKIP_CHROMIUM=1 skips the download entirely (dev only; release
 *         CI sets MIX_REQUIRE_CHROMIUM=1 to fail closed instead of shipping
 *         browser-less).
 *         MIX_REQUIRE_CHROMIUM=1 fails when the bundle would be skipped/missing.
 *         MIX_CHROMIUM_CHANNEL=beta|canary|dev picks a non-stable channel.
 *         MIX_CHROMIUM_VERSION=<x> pins the version (CI sets this).
 *         MIX_CHROMIUM_CFT=<platform> pins the Chrome-for-Testing platform
 *         (mac-arm64 / mac-x64 / win32 / win64 / win-arm64 / linux64). The
 *         release matrix cross-builds several targets from one runner, so the
 *         runner's own arch is not necessarily the bundle's arch; without
 *         this an Intel bundle would ship an arm64 Chromium, or the reverse.
 *         MIX_CHROMIUM_EXPECTED_VERSION=<x> refuses to float past this version:
 *         when MIX_CHROMIUM_VERSION is unset, the resolved latest-good must
 *         equal this value or the fetch aborts. Set it to the last reviewed
 *         version so an unpinned local/dev run fails loudly instead of
 *         silently shipping a newer Chromium (release CI always pins
 *         MIX_CHROMIUM_VERSION, so this is a dev-only backstop).
 *         MIX_CHROMIUM_SHA256 / MIX_CHROMIUM_SIZE verify the zip. Each CfT
 *         platform ships its own zip, so CI sets these per target
 *         (MIX_CHROMIUM_SHA256_MAC_X64 and friends).
 */
import { existsSync } from "node:fs";
import { chmod, mkdir, readdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import os from "node:os";

const here = dirname(fileURLToPath(import.meta.url));
const outRoot = resolve(here, "..", "app", "resources", "chromium");
const versionFile = join(outRoot, "chromium-version.json");
const args = process.argv.slice(2);
const force = args.includes("--force");
const pinned = args.includes("--version")
  ? args[args.indexOf("--version") + 1]
  : process.env.MIX_CHROMIUM_VERSION ?? null;
// Explicit Chrome-for-Testing platform, e.g. "mac-x64" for an Intel macOS
// bundle built on an Apple Silicon runner. The release matrix builds several
// targets from the same runner label, so the runner's arch is not necessarily
// the bundle's arch.
const cftPinned = args.includes("--cft")
  ? args[args.indexOf("--cft") + 1]
  : process.env.MIX_CHROMIUM_CFT ?? null;

/** @typedef {"mac"|"win"|"linux"} Platform */

/** CfT platform ids Chrome for Testing actually publishes. */
const CFT_PLATFORMS = ["mac-arm64", "mac-x64", "win32", "win64", "win-arm64", "linux64"];

function detectPlatform() {
  if (os.platform() === "darwin") return "mac";
  if (os.platform() === "win32") return "win";
  return "linux";
}

/** Chrome for Testing platform identifier, which folds in the CPU arch.
 * NOTE: win-arm64 maps to "win-arm64" (not "win64"); CfT ships
 * chrome-win-arm64.zip for ARM Windows. Untested in CI (no win-arm64 runner).
 *
 * An explicit pin (MIX_CHROMIUM_CFT / --cft) wins over the runner's arch:
 * a cross-built bundle needs the *target* arch, not the host's.
 */
function cftPlatform(platform) {
  if (cftPinned) {
    if (!CFT_PLATFORMS.includes(cftPinned)) {
      throw new Error(
        `[chromium] unknown CfT platform "${cftPinned}" (expected one of ${CFT_PLATFORMS.join(", ")})`,
      );
    }
    const prefix = platform === "mac" ? "mac" : platform === "win" ? "win" : "linux";
    if (!cftPinned.startsWith(prefix)) {
      throw new Error(
        `[chromium] CfT platform "${cftPinned}" does not match the host platform "${platform}"`,
      );
    }
    return cftPinned;
  }
  const arm = os.arch() === "arm64";
  if (platform === "mac") return arm ? "mac-arm64" : "mac-x64";
  if (platform === "win") return arm ? "win-arm64" : "win32";
  if (arm) {
    // CfT ships linux64 only for the bundled path; arm64-linux is deferred
    // (see desktop-release.yml matrix comments). Shipping the x64 binary on
    // arm64 would produce a bundle that cannot launch, so fail with a clear
    // pointer at the system-Chromium fallback instead.
    throw new Error(
      "[chromium] linux/arm64 has no Chrome-for-Testing bundle; " +
      "rebuild with MIX_SKIP_CHROMIUM=1 to fall back to a system Chromium.",
    );
  }
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
  // Chrome for Testing ships chrome-mac-arm64 / chrome-mac-x64 /
  // chrome-win32 / chrome-win64 / chrome-win-arm64 / chrome-linux64.
  return `chrome-${cft}`;
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
  const expected = process.env.MIX_CHROMIUM_EXPECTED_VERSION ?? null;
  // Unpinned builds float with the channel's latest-good. Say so loudly: a
  // future rebuild may ship a different Chromium without any local change.
  // CI pins MIX_CHROMIUM_VERSION (see desktop-release.yml env). Set
  // MIX_CHROMIUM_EXPECTED_VERSION to the last reviewed version to refuse to
  // float past it (dev-only backstop; release CI never floats).
  console.warn(
    "[chromium] WARNING: MIX_CHROMIUM_VERSION is not set — resolving the latest good build. " +
    "This build floats: a future rebuild may ship a different Chromium without any local change. " +
    "Pin it (MIX_CHROMIUM_VERSION=<x> or --version <x>) for reproducible builds. " +
    "To refuse floating past a reviewed version, set MIX_CHROMIUM_EXPECTED_VERSION=<x>.",
  );
  const channel = process.env.MIX_CHROMIUM_CHANNEL ?? "Stable";
  const url = `https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json`;
  console.log(`[chromium] resolving ${channel} from chrome-for-testing…`);
  const res = await fetchWithRetry(url);
  if (!res.ok) throw new Error(`version lookup failed: ${res.status} ${res.statusText}`);
  const json = await res.json();
  const entry = json.channels?.[channel];
  if (!entry?.version) throw new Error(`no "${channel}" channel in chrome-for-testing response`);
  if (expected && entry.version !== expected) {
    throw new Error(
      `resolved Chromium ${entry.version} != expected ${expected} ` +
      `(chromium-version.json expectation). Refusing to float.`,
    );
  }
  return entry.version;
}

async function fetchWithRetry(url, options, attempts = 3) {
  let last;
  for (let i = 1; i <= attempts; i++) {
    try {
      return await fetch(url, options);
    } catch (err) {
      last = err;
      if (i < attempts) {
        const backoff = 1000 * 2 ** (i - 1);
        console.warn(`[chromium] fetch attempt ${i}/${attempts} failed (${err.message}); retrying in ${backoff}ms…`);
        await new Promise((r) => setTimeout(r, backoff));
      }
    }
  }
  throw last;
}

async function download(url, dest, version, cft) {
  console.log(`[chromium] GET ${url}`);
  let last;
  for (let i = 1; i <= 3; i++) {
    try {
      const res = await fetch(url, { redirect: "follow" });
      if (!res.ok) throw new Error(`download failed: ${res.status} ${res.statusText}`);
      const buf = Buffer.from(await res.arrayBuffer());
      await verifyArchive(buf, version, cft);
      await writeFile(dest, buf);
      return buf.length;
    } catch (err) {
      last = err;
      if (i < 3) {
        const backoff = 1000 * 2 ** (i - 1);
        console.warn(`[chromium] download attempt ${i}/3 failed (${err.message}); retrying in ${backoff}ms…`);
        await new Promise((r) => setTimeout(r, backoff));
      }
    }
  }
  throw last;
}

/** Verify size/SHA-256 against explicit env expectations (CI checksum gate).
 * MIX_CHROMIUM_SHA256 / MIX_CHROMIUM_SIZE guard against truncated mirrors.
 * When unset (local dev), the download is accepted as-is.
 * TODO: verify a code signature after download (CfT provides no detached
 * signatures today, so SHA-256 pinning is the current supply-chain anchor;
 * upgrade to signature verification if upstream publishes signing keys). */
async function verifyArchive(buf, version, cft) {
  const wantSha = process.env.MIX_CHROMIUM_SHA256 ?? null;
  const wantSize = process.env.MIX_CHROMIUM_SIZE ? Number(process.env.MIX_CHROMIUM_SIZE) : null;
  if (wantSize != null && Number.isFinite(wantSize) && buf.length !== wantSize) {
    throw new Error(`checksum mismatch: got ${buf.length} bytes, expected ${wantSize} (version ${version} ${cft})`);
  }
  if (wantSha) {
    const { createHash } = await import("node:crypto");
    const got = createHash("sha256").update(buf).digest("hex");
    if (got !== wantSha.toLowerCase()) {
      throw new Error(`SHA-256 mismatch for ${version} ${cft}: got ${got}, expected ${wantSha}`);
    }
    console.log(`[chromium] SHA-256 verified: ${got.slice(0, 16)}…`);
  }
}

async function unpack(archive, dest) {
  await mkdir(dest, { recursive: true });
  // NOTE: `execFileSync` takes an argv array (no shell), so `archive`/`dest`
  // are never shell-interpreted. The PowerShell `-Command` string below is
  // the one exception: single quotes are escaped by doubling them so paths
  // with `'` (or spaces) cannot break out of the literal.
  const psQuote = (s) => `'${String(s).replaceAll("'", "''")}'`;
  try {
    if (os.platform() === "win32") {
      execFileSync(
        "powershell",
        ["-NoProfile", "-Command",
          `Expand-Archive -Force -LiteralPath ${psQuote(archive)} -DestinationPath ${psQuote(dest)}`],
        { stdio: "inherit" },
      );
    } else {
      execFileSync("unzip", ["-q", "-o", archive, "-d", dest], { stdio: "inherit" });
    }
  } catch (err) {
    const tool = os.platform() === "win32" ? "Expand-Archive (PowerShell)" : "unzip";
    throw new Error(
      `extraction failed: ${tool} is missing or errored (${err.message}). ` +
      `Install unzip (macOS/Linux) or use PowerShell 5+ (Windows), then re-run.`,
    );
  }
}

async function main() {
  if (process.env.MIX_SKIP_CHROMIUM === "1") {
    if (process.env.MIX_REQUIRE_CHROMIUM === "1") {
      console.error("[chromium] MIX_SKIP_CHROMIUM=1 conflicts with MIX_REQUIRE_CHROMIUM=1 — refusing to ship browser-less.");
      process.exit(2);
    }
    console.log("[chromium] MIX_SKIP_CHROMIUM=1 — skipping (app falls back to system Chromium; NOT for release builds)");
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

  // Skip when the recorded version already matches: the binary check above
  // only proves *some* build is present, the version file proves it is the
  // one we would fetch now.
  if (!force) {
    try {
      const recorded = JSON.parse(await readFile(versionFile, "utf8"));
      if (recorded?.version === version && (await exists(dest))) {
        console.log(`[chromium] already present: ${dest} (version ${version})`);
        return;
      }
    } catch {
      // No (or unreadable) version file — fall through to the download.
    }
  }

  const cdl = `https://storage.googleapis.com/chrome-for-testing-public/${version}/${cft}/chrome-${cft}.zip`;
  const tmp = join(os.tmpdir(), `mix-chromium-${version}-${cft}.zip`);

  const bytes = await download(cdl, tmp, version, cft);
  console.log(`[chromium] ${(bytes / 1e6).toFixed(1)} MB, extracting…`);

  // Atomic install: unpack into a staging dir and only swap it into place on
  // success. The old code `rm -rf`'d `outRoot` first, so a failed download or
  // extraction left the tree with no Chromium at all (and no version file).
  const staging = `${outRoot}.new-${process.pid}`;
  await rm(staging, { recursive: true, force: true });
  await mkdir(staging, { recursive: true });
  try {
    await unpack(tmp, staging);
  } catch (err) {
    await rm(staging, { recursive: true, force: true }).catch(() => {});
    throw err;
  }
  await rm(tmp, { force: true });

  // Normalise the archive's own directory name to the layout the Rust resolver
  // looks for, so we do not have to duplicate Chrome for Testing's naming.
  // All of this happens inside the staging dir; the live tree is untouched
  // until the swap below.
  const stageRoot = staging;
  const stageDest = dest.replace(outRoot, stageRoot);
  const stageVersionFile = versionFile.replace(outRoot, stageRoot);
  const extracted = join(stageRoot, archiveRoot(cft));
  const normalized =
    platform === "mac" ? join(stageRoot, "chrome-mac")
      : platform === "win" ? join(stageRoot, "chrome-win")
        : join(stageRoot, "chrome-linux");
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

  if (platform !== "win") await chmod(stageDest, 0o755);

  if (!existsSync(stageDest)) {
    const listing = await readdir(stageRoot).catch(() => []);
    await rm(staging, { recursive: true, force: true }).catch(() => {});
    throw new Error(`extraction did not produce ${stageDest}\nfound: ${listing.join(", ")}`);
  }
  await writeFile(
    stageVersionFile,
    JSON.stringify(
      { version, channel: process.env.MIX_CHROMIUM_CHANNEL ?? "Stable", pinned: pinned ?? null, platform: cft, fetchedAt: new Date().toISOString() },
      null,
      2,
    ) + "\n",
  );
  // Swap into place: only now does the previous tree get replaced, so an
  // interrupted run can never leave a half-written `resources/chromium`.
  await rm(outRoot, { recursive: true, force: true });
  await rename(staging, outRoot);
  console.log(`[chromium] ready: ${dest} (version ${version}, recorded in chromium-version.json)`);
}

main().catch((err) => {
  console.error(`[chromium] ${err.message}`);
  if (process.env.MIX_REQUIRE_CHROMIUM === "1") {
    console.error("[chromium] MIX_REQUIRE_CHROMIUM=1 — failing closed; refusing browser-less build.");
    process.exit(2);
  }
  // Dev fallback: the app runs against a system Chromium, so a failed fetch
  // is a warning, not a build break. (CI sets MIX_REQUIRE_CHROMIUM=1 to fail
  // closed instead.) Exit 0 so `beforeBuildCommand` does not fail for it.
  console.warn("[chromium] continuing without a bundled Chromium — the app falls back to a system Chromium (dev only).");
  console.warn("[chromium] to skip this step entirely, rebuild with MIX_SKIP_CHROMIUM=1.");
  process.exit(0);
});
