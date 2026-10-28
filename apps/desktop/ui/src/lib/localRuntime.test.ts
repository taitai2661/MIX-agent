import { describe, expect, it } from "vitest";
import {
  isLocalBaseUrl,
  isLocalHost,
  isLoopbackBaseUrl,
  localRuntimeForModel,
} from "./localRuntime";
import type { ModelView, ProviderView } from "./types";

function model(providerId: string): ModelView {
  return {
    id: `${providerId}:qwen3:8b`,
    provider_id: providerId,
    label: "qwen3:8b",
    context_window: 8192,
    supports_vision: false,
    supports_tool_calling: true,
    supports_reasoning: true,
    auto_candidate: true,
  };
}

function provider(
  id: string,
  baseUrl: string,
  preset = "ollama",
): ProviderView {
  return {
    id,
    kind: "ollama",
    label: "Ollama",
    base_url: baseUrl,
    enabled: true,
    has_api_key: false,
    preset,
  };
}

describe("isLocalBaseUrl", () => {
  it("accepts loopback, LAN, mDNS and container aliases", () => {
    for (const url of [
      "http://127.0.0.1:11434/v1",
      "http://localhost:1234/v1",
      "http://[::1]:8000/v1",
      "http://192.168.1.20:11434/v1",
      "http://10.0.0.5:11434/v1",
      "http://172.20.0.1:11434/v1",
      "http://100.100.0.1:11434/v1",
      "http://mac-mini.local:11434/v1",
      "http://host.docker.internal:11434/v1",
      "http://localhost",
    ]) {
      expect(isLocalBaseUrl(url), url).toBe(true);
    }
  });

  it("rejects cloud endpoints and adjacent private ranges", () => {
    for (const url of [
      "https://api.openai.com/v1",
      "https://ollama.example.com/v1",
      "https://172.15.0.1/v1",
      "https://172.32.0.1/v1",
      "https://100.63.0.1/v1",
      "",
      null,
    ]) {
      expect(isLocalBaseUrl(url), String(url)).toBe(false);
    }
  });

  it("normalises bare hosts", () => {
    expect(isLocalHost("LOCALHOST")).toBe(true);
    expect(isLocalHost("[::1]")).toBe(true);
  });

  it("treats only loopback as a managed runtime", () => {
    for (const url of [
      "http://127.0.0.1:11434/v1",
      "http://localhost:1234/v1",
      "http://[::1]:8000/v1",
    ]) {
      expect(isLoopbackBaseUrl(url), url).toBe(true);
      expect(isLocalBaseUrl(url), url).toBe(true);
    }
    for (const url of [
      "http://192.168.1.20:11434/v1",
      "http://10.0.0.5:11434/v1",
      "http://mac-mini.local:11434/v1",
      "http://100.100.0.1:11434/v1",
    ]) {
      expect(isLoopbackBaseUrl(url), url).toBe(false);
      // Still local for the API-key gate.
      expect(isLocalBaseUrl(url), url).toBe(true);
    }
  });
});

describe("localRuntimeForModel", () => {
  it("returns the runtime slug for a loopback provider", () => {
    expect(
      localRuntimeForModel(
        "local:qwen3:8b",
        [model("local")],
        [provider("local", "http://127.0.0.1:11434/v1")],
      ),
    ).toBe("ollama");
  });

  it("ignores an Ollama preset pointed at a remote host", () => {
    expect(
      localRuntimeForModel(
        "gpu:qwen3:8b",
        [model("gpu")],
        [provider("gpu", "http://192.0.2.10:11434/v1")],
      ),
    ).toBeNull();
  });

  it("does not manage a LAN Ollama (it is local, not ours to spawn)", () => {
    expect(
      localRuntimeForModel(
        "lan:qwen3:8b",
        [model("lan")],
        [provider("lan", "http://192.168.1.20:11434/v1")],
      ),
    ).toBeNull();
  });

  it("does not treat non-runtime local servers as managed runtimes", () => {
    expect(
      localRuntimeForModel(
        "custom:qwen3:8b",
        [model("custom")],
        [provider("custom", "http://127.0.0.1:8080/v1", "custom")],
      ),
    ).toBeNull();
  });

  it("returns null for the Auto pseudo-model", () => {
    expect(
      localRuntimeForModel(
        "auto",
        [model("p")],
        [provider("p", "http://127.0.0.1:1/v1")],
      ),
    ).toBeNull();
  });
});
