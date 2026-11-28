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
// Ratcheted to 520 kB for v0.5.8: the shell now ships notification
// navigation, palette recency, the surface switcher and the approval drawer
// trigger on the startup path (~511 kB measured). Panels remain lazy-loaded.
const MAIN_BUDGET = 520 * 1024;
const TOTAL_BUDGET = 1_500 * 1024;

if (!fs.existsSync(root)) {
  console.error(`bundle budget: ${root} does not exist; run vite build first`);
  process.exit(2);
}

// Count every asset the WebView actually loads: JS chunks, emitted CSS and
// static files. Source maps are build/debug artifacts, not shipped payload, so
// they are reported separately instead of being added to the budget.
const files = fs
  .readdirSync(root)
  .map((name) => ({ name, path: path.join(root, name) }))
  .filter((entry) => fs.statSync(entry.path).isFile())
  .map((entry) => ({ name: entry.name, bytes: fs.statSync(entry.path).size }));
const sourceMaps = files.filter((entry) => entry.name.endsWith(".map"));
const entries = files.filter((entry) => !entry.name.endsWith(".map"));
const jsEntries = entries.filter((entry) => entry.name.endsWith(".js"));
const cssBytes = entries.filter((entry) => entry.name.endsWith(".css")).reduce((sum, e) => sum + e.bytes, 0);
const otherBytes = entries
  .filter((entry) => !entry.name.endsWith(".js") && !entry.name.endsWith(".css"))
  .reduce((sum, e) => sum + e.bytes, 0);
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
  ? jsEntries.find((entry) => entry.name === path.basename(entryPath))
  : undefined;

const kb = (bytes) => `${(bytes / 1024).toFixed(1)} kB`;
console.log(`bundle budget: initial ${main ? kb(main.bytes) : "missing"}`);
console.log(
  `bundle budget: total ${kb(total)} ` +
    `(JS ${kb(total - cssBytes - otherBytes)}, CSS ${kb(cssBytes)}, other ${kb(otherBytes)}; ` +
    `excluded ${sourceMaps.length} source map(s), ${kb(sourceMaps.reduce((sum, e) => sum + e.bytes, 0))})`,
);

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
    `bundle budget: total emitted assets ${kb(total)} exceeds ${kb(TOTAL_BUDGET)}`,
  );
  failed = true;
}
process.exit(failed ? 1 : 0);
