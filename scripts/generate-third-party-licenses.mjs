#!/usr/bin/env node
/**
 * scripts/generate-third-party-licenses.mjs
 *
 * Scans dependencies across the monorepo:
 * - Rust crates (apps/desktop/Cargo.lock)
 * - Python packages (apps/server/uv.lock)
 * - Node.js packages (the root, web, desktop and desktop UI package.json files,
 *   with versions resolved from their pnpm lockfiles)
 *
 * Outputs a consolidated THIRD_PARTY_LICENSES.md for distribution compliance
 * and licensing audits. The output is deterministic: all tables are sorted and
 * versions come from committed lockfiles, never from an installed node_modules.
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");

function parseCargoLock() {
  const path = resolve(ROOT, "apps/desktop/Cargo.lock");
  if (!existsSync(path)) return [];
  const content = readFileSync(path, "utf8");
  const pkgs = [];
  const matches = content.matchAll(/\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"/g);
  for (const m of matches) {
    if (!m[1].startsWith("mix-agent") && !m[1].startsWith("mixd-")) {
      pkgs.push({ name: m[1], version: m[2], type: "Rust (Cargo)" });
    }
  }
  return pkgs;
}

function parseUvLock() {
  const path = resolve(ROOT, "apps/server/uv.lock");
  if (!existsSync(path)) return [];
  const content = readFileSync(path, "utf8");
  const pkgs = [];
  const matches = content.matchAll(/\[\[package\]\][\s\S]*?name = "([^"]+)"\s*\nversion = "([^"]+)"/g);
  for (const m of matches) {
    if (m[1] !== "mix-agent") {
      pkgs.push({ name: m[1], version: m[2], type: "Python (uv)" });
    }
  }
  return pkgs;
}

/** Unquote a pnpm-lock.yaml key (quoted scoped names are common). */
function unquoteKey(raw) {
  return raw.startsWith("'") && raw.endsWith("'") || raw.startsWith('"') && raw.endsWith('"')
    ? raw.slice(1, -1)
    : raw;
}

/**
 * Map importer -> (package name -> resolved version) from a pnpm lockfile.
 * Only the `importers:` block is read: the `packages:`/`snapshots:` blocks
 * contain transitive entries that are not direct dependencies of a manifest.
 */
function parseNodeLock(relativePath) {
  const path = resolve(ROOT, relativePath);
  if (!existsSync(path)) return new Map();
  const importers = new Map();
  let inImporters = false;
  let importer = null;
  let inDeps = false;
  let pendingName = null;
  for (const line of readFileSync(path, "utf8").split(/\r?\n/)) {
    if (/^\S/.test(line)) {
      // A new top-level YAML section (packages:, snapshots:, ...).
      inImporters = /^importers:\s*$/.test(line);
      importer = null;
      inDeps = false;
      pendingName = null;
      continue;
    }
    if (!inImporters) continue;
    const importerMatch = line.match(/^ {2}(?:'([^']+)'|"([^"]+)"|([^\s:#]+)):\s*$/);
    if (importerMatch) {
      importer = unquoteKey(importerMatch[1] ?? importerMatch[2] ?? importerMatch[3]);
      if (!importers.has(importer)) importers.set(importer, new Map());
      inDeps = false;
      pendingName = null;
      continue;
    }
    if (/^ {4}(?:dependencies|devDependencies):\s*$/.test(line)) {
      inDeps = true;
      pendingName = null;
      continue;
    }
    if (/^ {4}\S/.test(line)) {
      // A different importer-level key (specifier, dependenciesMeta, ...).
      inDeps = false;
      pendingName = null;
      continue;
    }
    if (!inDeps || importer === null) continue;
    const nameMatch = line.match(/^ {6}(?:'([^']+)'|"([^"]+)"|([^\s:#]+)):\s*$/);
    if (nameMatch) {
      pendingName = unquoteKey(nameMatch[1] ?? nameMatch[2] ?? nameMatch[3]);
      continue;
    }
    if (pendingName === null) continue;
    const versionMatch = line.match(/^ {8}version:\s*(\S+)/);
    if (!versionMatch) continue;
    const raw = versionMatch[1];
    // Peer-dependency suffixes are not part of the resolved package version.
    // Workspace links are not third-party packages.
    const version = raw.split("(", 1)[0];
    if (!raw.startsWith("link:") && version) importers.get(importer).set(pendingName, version);
    pendingName = null;
  }
  return importers;
}

/** Each manifest's dependencies, resolved with its own lockfile + importer. */
const NODE_MANIFESTS = [
  { manifest: "package.json", lockfile: "pnpm-lock.yaml", importer: "." },
  { manifest: "apps/web/package.json", lockfile: "apps/web/pnpm-lock.yaml", importer: "." },
  { manifest: "apps/desktop/package.json", lockfile: "apps/desktop/pnpm-lock.yaml", importer: "." },
  { manifest: "apps/desktop/ui/package.json", lockfile: "apps/desktop/pnpm-lock.yaml", importer: "ui" },
];

function parseNodePackages() {
  const byName = new Map();
  const lockCache = new Map();
  for (const { manifest, lockfile, importer } of NODE_MANIFESTS) {
    const manifestPath = resolve(ROOT, manifest);
    if (!existsSync(manifestPath)) continue;
    if (!lockCache.has(lockfile)) lockCache.set(lockfile, parseNodeLock(lockfile));
    const resolutions = lockCache.get(lockfile).get(importer) ?? new Map();
    const json = JSON.parse(readFileSync(manifestPath, "utf8"));
    const deps = { ...json.dependencies, ...json.devDependencies };
    for (const name of Object.keys(deps)) {
      const version = resolutions.get(name);
      if (!version) {
        throw new Error(
          `${manifest}: ${name} (${deps[name]}) has no resolved version in ${lockfile} ` +
            `importer "${importer}"; regenerate that lockfile before generating notices`,
        );
      }
      if (!byName.has(name)) byName.set(name, new Set());
      byName.get(name).add(version);
    }
  }
  return Array.from(byName, ([name, versions]) => ({
    name,
    version: [...versions].sort((a, b) => a.localeCompare(b, "en", { numeric: true })).join(", "),
    type: "Node.js (npm)",
  }));
}

function generateMarkdown() {
  const rust = parseCargoLock();
  const python = parseUvLock();
  const node = parseNodePackages();

  const lines = [
    "# Third-Party Open Source Software Notices",
    "",
    "This document lists third-party software components and dependencies used by MIX-agent.",
    "",
    "## 1. Overview",
    "",
    `- **Rust Dependencies**: ${rust.length} crates`,
    `- **Python Dependencies**: ${python.length} packages`,
    `- **Node.js / Web Dependencies**: ${node.length} packages`,
    "",
    "## 2. Special Notices & Disclosures",
    "",
    "### GitHub Octicons (Docker Edition)",
    "- **License**: MIT License (Copyright (c) 2018 GitHub Inc.)",
    "- **Notice**: See `apps/MIX-agent/OCTICONS-LICENSE` and `apps/web/public/OCTICONS-LICENSE`.",
    "",
    "### Python psycopg (PostgreSQL Driver)",
    "- **License**: GNU Lesser General Public License v3.0 or later (LGPL-3.0-or-later)",
    "- **Scope**: Used server-side via dynamic import in `apps/server`. Under LGPL-3.0, running the server in containerized/self-hosted environments does not impose copyleft obligations on MIX-agent proprietary/Apache-2.0 code.",
    "",
    "### Python certifi",
    "- **License**: Mozilla Public License 2.0 (MPL-2.0)",
    "- **Scope**: File-level copyleft certificate bundle; unaltered runtime dependency.",
    "",
    "## 3. Python Server Dependencies",
    "",
    "| Package | Version | Ecosystem |",
    "| :--- | :--- | :--- |",
  ];

  for (const p of python.sort((a, b) => a.name.localeCompare(b.name))) {
    lines.push(`| \`${p.name}\` | \`${p.version}\` | Python |`);
  }

  lines.push(
    "",
    "## 4. Web & Desktop UI Dependencies",
    "",
    "Versions are resolved from the workspace lockfiles, not the semver ranges in `package.json`.",
    "Where a package resolves to different versions in different workspaces, all are listed.",
    "",
    "| Package | Version | Ecosystem |",
    "| :--- | :--- | :--- |",
  );
  for (const p of node.sort((a, b) => a.name.localeCompare(b.name))) {
    lines.push(`| \`${p.name}\` | \`${p.version}\` | npm |`);
  }

  lines.push("", "## 5. Desktop Rust Crates (Summary)", "", `Total: ${rust.length} crates compiled into Desktop binaries.`, "", "<details>", "<summary>Expand full crate list</summary>", "", "| Crate | Version | Ecosystem |", "| :--- | :--- | :--- |");
  for (const p of rust.sort((a, b) => a.name.localeCompare(b.name))) {
    lines.push(`| \`${p.name}\` | \`${p.version}\` | Rust |`);
  }
  lines.push("</details>", "");

  return lines.join("\n");
}

const md = generateMarkdown();
const outPath = resolve(ROOT, "THIRD_PARTY_LICENSES.md");
writeFileSync(outPath, md, "utf8");
console.log(`[licenses] generated ${outPath}`);

const desktopOutPath = resolve(ROOT, "apps/desktop/THIRD_PARTY_LICENSES.md");
writeFileSync(desktopOutPath, md, "utf8");
console.log(`[licenses] generated ${desktopOutPath}`);
