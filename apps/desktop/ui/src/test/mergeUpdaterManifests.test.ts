import { describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mergeManifests } from "../../../scripts/merge-updater-manifests.mjs";

describe("mergeManifests", () => {
  it("combines per-platform latest.json manifests into a single unified manifest", () => {
    const tmp = mkdtempSync(join(tmpdir(), "updater-test-"));
    try {
      const macDir = join(tmp, "mix-agent-desktop-aarch64-apple-darwin");
      const winDir = join(tmp, "mix-agent-desktop-x86_64-pc-windows-msvc");
      mkdirSync(macDir, { recursive: true });
      mkdirSync(winDir, { recursive: true });

      writeFileSync(
        join(macDir, "latest.json"),
        JSON.stringify({
          version: "0.2.0",
          notes: "Mac release notes",
          pub_date: "2026-10-25T00:00:00Z",
          platforms: {
            "darwin-aarch64": {
              signature: "mac-sig",
              url: "mix-agent-desktop_0.2.0_aarch64.app.tar.gz",
            },
          },
        })
      );

      writeFileSync(
        join(winDir, "latest.json"),
        JSON.stringify({
          version: "0.2.0",
          notes: "Win release notes",
          pub_date: "2026-10-25T00:00:00Z",
          platforms: {
            "windows-x86_64": {
              signature: "win-sig",
              url: "mix-agent-desktop_0.2.0_x64-setup.nsis.zip",
            },
          },
        })
      );

      const outFile = join(tmp, "latest.json");
      const result = mergeManifests(tmp, outFile, {
        repo: "taitai2661/MIX-agent",
        tag: "v0.2.0",
        cleanupSubManifests: false,
      });

      expect(result).not.toBeNull();
      expect(result?.version).toBe("0.2.0");
      expect(result?.platforms["darwin-aarch64"]).toBeDefined();
      expect(result?.platforms["darwin-aarch64"].signature).toBe("mac-sig");
      expect(result?.platforms["darwin-aarch64"].url).toBe(
        "https://github.com/taitai2661/MIX-agent/releases/download/v0.2.0/mix-agent-desktop_0.2.0_aarch64.app.tar.gz"
      );
      expect(result?.platforms["windows-x86_64"]).toBeDefined();
      expect(result?.platforms["windows-x86_64"].signature).toBe("win-sig");
      expect(result?.platforms["windows-x86_64"].url).toBe(
        "https://github.com/taitai2661/MIX-agent/releases/download/v0.2.0/mix-agent-desktop_0.2.0_x64-setup.nsis.zip"
      );

      const saved = JSON.parse(readFileSync(outFile, "utf-8"));
      expect(saved.version).toBe("0.2.0");
      expect(saved.platforms["darwin-aarch64"]).toBeDefined();
      expect(saved.platforms["windows-x86_64"]).toBeDefined();
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });
});
