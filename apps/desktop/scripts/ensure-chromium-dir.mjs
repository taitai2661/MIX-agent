#!/usr/bin/env node
/**
 * Make sure the `resources/chromium` directory that `bundle.resources`
 * declares in `app/tauri.conf.json` exists before `tauri dev` runs.
 *
 * `tauri-build` validates every declared resource path while generating the
 * app manifest and fails the whole build when one is missing
 * (`resource path 'resources/chromium' doesn't exist`). The directory is a
 * `.gitignore`d build artifact fetched by `fetch-chromium.mjs`, which only runs
 * as part of `beforeBuildCommand` — so a clean checkout cannot `tauri dev`.
 *
 * Dev runs do not need the ~370 MB bundle: the Rust side
 * (`mixd_browser::profile::find_bundled_chromium` / `find_system_chromium`)
 * falls back to a system Chromium. So we just create the placeholder
 * directory and let dev proceed. `fetch-chromium.mjs` fills it in for real
 * during `tauri build`.
 *
 * Usage:  node scripts/ensure-chromium-dir.mjs
 */
import { mkdir, stat } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const outRoot = resolve(here, "..", "app", "resources", "chromium");

async function exists(path) {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

async function main() {
  if (process.env.MIX_REQUIRE_CHROMIUM === "1" && !(await exists(outRoot))) {
    console.error(
      `[chromium-dir] MIX_REQUIRE_CHROMIUM=1 but ${outRoot} is missing — run fetch-chromium.mjs first.`,
    );
    process.exit(2);
  }
  if (await exists(outRoot)) {
    console.log(`[chromium-dir] present: ${outRoot}`);
    return;
  }
  await mkdir(outRoot, { recursive: true });
  console.log(`[chromium-dir] created placeholder: ${outRoot}`);
  console.log(
    "[chromium-dir] dev falls back to a system Chromium; release builds set " +
    "MIX_REQUIRE_CHROMIUM=1 and fail closed when the bundle is missing.",
  );
}

main().catch((err) => {
  console.error(`[chromium-dir] ${err.message}`);
  process.exit(1);
});
