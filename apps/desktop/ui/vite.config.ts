/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "es2022",
    sourcemap: false,
    // Budget: the desktop WebView loads from disk, but the chat path still
    // owns the startup experience — keep the initial chunk small and push
    // everything else into named async chunks (see the `lazy` panes in
    // App.tsx). Warn at 600 kB per chunk; the main chunk sits just under it
    // (~590 kB: radix + tauri plugins + lucide icons + app code) and the
    // markdown chunk (~160 kB) only loads with the transcript.
    // `reportCompressedSize` reports gzip sizes (the honest number for
    // update-payload cost). Gate: `chunkSizeWarningLimit` fails CI's
    // size check — do not raise it without moving the weight into a lazy
    // chunk first (see `manualChunks` below and the `lazy` panes in App.tsx).
    chunkSizeWarningLimit: 600,
    reportCompressedSize: true,
    rollupOptions: {
      output: {
        manualChunks: {
          // Markdown rendering (react-markdown + remark-gfm + remark-breaks)
          // is only needed by the transcript and file previews.
          markdown: ["react-markdown", "remark-gfm", "remark-breaks"],
          vendor: ["react", "react-dom", "@tanstack/react-query", "zustand"],
          radix: [
            "@radix-ui/react-dialog",
            "@radix-ui/react-dropdown-menu",
            "@radix-ui/react-popover",
            "@radix-ui/react-slot",
          ],
          icons: ["lucide-react"],
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
      // Floor, not ceiling: new code should arrive tested, and a dip below
      // the line fails CI so drift is caught at the PR, not in review.
      thresholds: {
        lines: 70,
        functions: 70,
        branches: 60,
        statements: 70,
      },
    },
  },
});
