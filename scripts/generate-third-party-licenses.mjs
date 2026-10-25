#!/usr/bin/env node
/**
 * scripts/generate-third-party-licenses.mjs
 *
 * Scans dependencies across the monorepo:
 * - Rust crates (apps/desktop/Cargo.lock)
 * - Python packages (apps/server/uv.lock)
 * - Node.js packages (apps/web/package.json & apps/desktop/ui/package.json)
 *
 * Outputs a consolidated THIRD_PARTY_LICENSES.md for distribution compliance
 * and licensing audits.
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

function parseNodePackages() {
  const files = [
    resolve(ROOT, "apps/web/package.json"),
    resolve(ROOT, "apps/desktop/ui/package.json"),
  ];
  const pkgs = new Map();
  for (const file of files) {
    if (!existsSync(file)) continue;
    const json = JSON.parse(readFileSync(file, "utf8"));
    const deps = { ...json.dependencies, ...json.devDependencies };
    for (const [name, version] of Object.entries(deps)) {
      if (!pkgs.has(name)) {
        pkgs.set(name, { name, version, type: "Node.js (npm)" });
      }
    }
  }
  return Array.from(pkgs.values());
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

  lines.push("", "## 4. Web & Desktop UI Dependencies", "", "| Package | Version | Ecosystem |", "| :--- | :--- | :--- |");
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
