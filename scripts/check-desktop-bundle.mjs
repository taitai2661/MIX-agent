#!/usr/bin/env node
// Bundle budget for the desktop WebView.
//
// Unlike latency, bundle size is deterministic across machines, so it is a
// safe CI gate. The initial shell chunk is what the first paint waits for;
// every other pane is deliberately lazy-loaded (see `App.tsx`).
import fs from "node:fs";
import path from "node:path";

const dist = path.resolve(
  path.dirname(new URL(import.meta.url).pathname),
  "..",
  "apps/desktop/ui/dist",
);
const root = path.join(dist, "assets");
const MAIN_BUDGET = 500 * 1024;
const TOTAL_BUDGET = 1_500 * 1024;

if (!fs.existsSync(root)) {
  console.error(`bundle budget: ${root} does not exist; run vite build first`);
  process.exit(2);
}

const entries = fs
  .readdirSync(root)
  .filter((name) => name.endsWith(".js"))
  .map((name) => ({
    name,
    bytes: fs.statSync(path.join(root, name)).size,
  }));
const total = entries.reduce((sum, entry) => sum + entry.bytes, 0);

// Read the entry from index.html instead of matching `index-*.js`. A lazy
// chunk can legitimately be emitted with an `index-` prefix (the
// @tauri-apps/plugin-os chunk is), and `find()` picked that 0.8 kB chunk while
// the real shell chunk was ~480 kB — so the gate stayed green regardless of
// the entry size.
const html = fs.readFileSync(path.join(dist, "index.html"), "utf8");
const entryPath =
  html.match(/<script[^>]*\btype="module"[^>]*\bsrc="([^"]+\.js)"/i)?.[1] ??
  html.match(/<script[^>]*\bsrc="([^"]+\.js)"/i)?.[1];
const main = entryPath
  ? entries.find((entry) => entry.name === path.basename(entryPath))
  : undefined;

const kb = (bytes) => `${(bytes / 1024).toFixed(1)} kB`;
console.log(`bundle budget: initial ${main ? kb(main.bytes) : "missing"}`);
console.log(`bundle budget: total ${kb(total)}`);

let failed = false;
if (!main) {
  console.error(
    `bundle budget: index.html does not reference an emitted entry chunk (looked for ${entryPath ?? "a module script"})`,
  );
  failed = true;
} else if (main.bytes > MAIN_BUDGET) {
  console.error(
    `bundle budget: initial chunk ${kb(main.bytes)} exceeds ${kb(MAIN_BUDGET)}`,
  );
  failed = true;
}
if (total > TOTAL_BUDGET) {
  console.error(
    `bundle budget: total JS ${kb(total)} exceeds ${kb(TOTAL_BUDGET)}`,
  );
  failed = true;
}
process.exit(failed ? 1 : 0);
