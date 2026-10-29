import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          react: ["react", "react-dom", "react-router-dom"],
          query: ["@tanstack/react-query"],
          markdown: ["react-markdown", "remark-gfm", "remark-breaks"],
          icons: ["lucide-react"],
        },
      },
    },
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:8080",
      "/health": "http://127.0.0.1:8080",
    },
  },
  test: {
    include: ["src/**/*.test.{ts,tsx}"],
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/generated/**", "src/**/*.test.{ts,tsx}"],
      // Floor, not ceiling. Set a little below the current measured value so
      // new code is pushed to arrive tested without failing on legacy files.
      // Ratchet: keep each metric at the measured value so new code must be
      // tested instead of relying on the large untested legacy surface.
      thresholds: {
        // Raised after the Chat/Composer split added interaction tests:
        // measured 41.2 / 60.36 / 79.9, kept slightly below to avoid flakes.
        lines: 40.5,
        functions: 60,
        branches: 79.5,
        statements: 40.5,
        // The API error envelope and its translations are a contract with the
        // server; a global 33% floor says nothing about them. Keep these high
        // so a new status code or message cannot land untested.
        "src/app/api.ts": {
          lines: 80,
          functions: 90,
          branches: 70,
          statements: 80,
        },
        "src/app/errorCodes.ts": {
          lines: 100,
          functions: 100,
          branches: 100,
          statements: 100,
        },
      },
    },
  },
});
