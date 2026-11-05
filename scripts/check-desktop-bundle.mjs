#!/usr/bin/env node
// Bundle budget for the desktop WebView.
//
// Unlike latency, bundle size is deterministic across machines, so it is a
// safe CI gate. The initial shell chunk is what the first paint waits for;
// every other pane is deliberately lazy-loaded (see `App.tsx`).
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(
  path.dirname(new URL(import.meta.url).pathname),
  "..",
  "apps/desktop/ui/dist/assets",
);
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
const main = entries.find((entry) => /^index-.*\.js$/.test(entry.name));

const kb = (bytes) => `${(bytes / 1024).toFixed(1)} kB`;
console.log(`bundle budget: initial ${main ? kb(main.bytes) : "missing"}`);
console.log(`bundle budget: total ${kb(total)}`);

let failed = false;
if (!main) {
  console.error("bundle budget: no index-*.js entry chunk was emitted");
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
