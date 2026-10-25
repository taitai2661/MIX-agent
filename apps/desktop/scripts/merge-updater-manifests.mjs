#!/usr/bin/env node
import { readdirSync, readFileSync, writeFileSync, statSync, unlinkSync } from "node:fs";
import { join, resolve, basename } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Recursively find files with exact name.
 */
function findFiles(dir, matchName, results = []) {
  if (!dir) return results;
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return results;
  }
  for (const entry of entries) {
    const full = join(dir, entry);
    try {
      const st = statSync(full);
      if (st.isDirectory()) {
        findFiles(full, matchName, results);
      } else if (entry === matchName) {
        results.push(full);
      }
    } catch {
      // ignore inaccessible files
    }
  }
  return results;
}

/**
 * Merges multiple platform latest.json manifests into a single multi-platform manifest.
 */
export function mergeManifests(searchDir, outputFile, options = {}) {
  const absSearchDir = resolve(searchDir);
  const absOutputFile = resolve(outputFile);

  console.log(`Searching for latest.json manifests under: ${absSearchDir}`);
  const manifestPaths = findFiles(absSearchDir, "latest.json").filter(
    (p) => resolve(p) !== absOutputFile
  );

  if (manifestPaths.length === 0) {
    console.warn("No latest.json manifests found to merge.");
    return null;
  }

  console.log(`Found ${manifestPaths.length} manifest(s):`);
  manifestPaths.forEach((p) => console.log(`  - ${p}`));

  let merged = {
    version: "",
    notes: "",
    pub_date: new Date().toISOString(),
    platforms: {},
  };

  for (const p of manifestPaths) {
    try {
      const raw = readFileSync(p, "utf-8");
      const json = JSON.parse(raw);
      if (!merged.version && json.version) merged.version = json.version;
      if (!merged.notes && json.notes) merged.notes = json.notes;
      if (json.pub_date) merged.pub_date = json.pub_date;

      if (json.platforms && typeof json.platforms === "object") {
        for (const [platform, data] of Object.entries(json.platforms)) {
          let targetData = { ...data };
          // If url is provided and repo+tag are given, format to release asset URL
          if (options.repo && options.tag && targetData.url) {
            const fileName = basename(targetData.url);
            targetData.url = `https://github.com/${options.repo}/releases/download/${options.tag}/${fileName}`;
          }
          merged.platforms[platform] = targetData;
        }
      }
    } catch (err) {
      console.error(`Error reading ${p}:`, err);
    }
  }

  const platformsCount = Object.keys(merged.platforms).length;
  console.log(`Successfully merged ${platformsCount} platform(s):`, Object.keys(merged.platforms));

  writeFileSync(absOutputFile, JSON.stringify(merged, null, 2) + "\n", "utf-8");
  console.log(`Wrote unified updater manifest to: ${absOutputFile}`);

  // Clean up subfolder latest.json files if requested so they don't collide
  if (options.cleanupSubManifests) {
    for (const p of manifestPaths) {
      try {
        unlinkSync(p);
      } catch (err) {
        console.warn(`Could not remove ${p}:`, err);
      }
    }
  }

  return merged;
}

const isDirectRun = process.argv[1] && (
  resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))
);

if (isDirectRun) {
  const args = process.argv.slice(2);
  const searchDir = args[0] || "artifacts";
  const outputFile = args[1] || join(searchDir, "latest.json");

  let repo = null;
  let tag = null;
  for (let i = 2; i < args.length; i++) {
    if (args[i] === "--repo" && args[i + 1]) repo = args[++i];
    if (args[i] === "--tag" && args[i + 1]) tag = args[++i];
  }

  mergeManifests(searchDir, outputFile, { repo, tag, cleanupSubManifests: true });
}
