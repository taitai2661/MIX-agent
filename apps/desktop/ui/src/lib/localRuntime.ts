import type { ModelView, ProviderView } from "./types";

/** Strip AAAA brackets so the checks below match the Rust `is_local_base_url`. */
function bareHost(host: string): string {
  return host.replace(/^\[/, "").replace(/\]$/, "").toLowerCase();
}

/** True when a host belongs to the user's own machine/network.
 *
 * Mirrors `mixd_providers::discovery::is_local_base_url`: loopback, RFC1918,
 * link-local, CGNAT/Tailscale, mDNS and the container host aliases. A provider
 * merely *named* Ollama but pointed at a cloud gateway must not be treated as a
 * local runtime.
 */
export function isLocalHost(host: string): boolean {
  const value = bareHost(host.trim());
  if (!value) return false;
  if (value === "localhost" || value === "::1" || value === "0.0.0.0") {
    return true;
  }
  if (
    value.startsWith("127.") ||
    value.startsWith("10.") ||
    value.startsWith("192.168.")
  ) {
    return true;
  }
  if (value.startsWith("172.")) {
    const second = Number(value.split(".")[1]);
    if (Number.isFinite(second) && second >= 16 && second <= 31) return true;
  }
  if (value.startsWith("100.")) {
    const second = Number(value.split(".")[1]);
    if (Number.isFinite(second) && second >= 64 && second <= 127) return true;
  }
  if (
    value === "host.docker.internal" ||
    value === "host.containers.internal" ||
    value === "gateway.docker.internal" ||
    value.endsWith(".local") ||
    value.endsWith(".localhost")
  ) {
    return true;
  }
  if (value.includes(":")) {
    const first = parseInt(value.split(":")[0], 16);
    if (Number.isFinite(first)) {
      if (
        (first >= 0xfc00 && first <= 0xfdff) ||
        (first >= 0xfe80 && first <= 0xfebf)
      ) {
        return true;
      }
    }
  }
  return false;
}

export function isLocalBaseUrl(baseUrl: string | null | undefined): boolean {
  if (!baseUrl || !baseUrl.trim()) return false;
  try {
    const url = new URL(
      baseUrl.includes("://") ? baseUrl : `http://${baseUrl}`,
    );
    return isLocalHost(url.hostname);
  } catch {
    return false;
  }
}

/** True only for this machine's loopback interface.
 *
 * Mirrors `mixd_providers::discovery::is_loopback_base_url`. The local-runtime
 * manager may only start/stop processes for these URLs: a LAN or Tailscale
 * server is "local" for the API-key gate, but spawning `ollama serve` for it
 * would start a second server on this machine.
 */
export function isLoopbackHost(host: string): boolean {
  const value = bareHost(host.trim());
  return (
    value === "localhost" ||
    value === "::1" ||
    value === "0.0.0.0" ||
    value.startsWith("127.")
  );
}

export function isLoopbackBaseUrl(baseUrl: string | null | undefined): boolean {
  if (!baseUrl || !baseUrl.trim()) return false;
  try {
    const url = new URL(
      baseUrl.includes("://") ? baseUrl : `http://${baseUrl}`,
    );
    return isLoopbackHost(url.hostname);
  } catch {
    return false;
  }
}

/**
 * The local runtime backing the selected model, when there is one.
 *
 * Requires the provider to actually point at a local address: a remote Ollama
 * on the LAN has a preset slug of `ollama` but is not a runtime this app can
 * start, stop, or sync models for.
 */
export function localRuntimeForModel(
  modelId: string,
  models: ModelView[],
  providers: ProviderView[],
): string | null {
  if (!modelId || modelId === "auto") return null;
  const model = models.find((entry) => entry.id === modelId);
  if (!model) return null;
  const provider = providers.find((entry) => entry.id === model.provider_id);
  // Lifecycle is loopback-only. A LAN/Tailscale Ollama is local for the key
  // gate but is not a process this app can start, stop or sync.
  if (!provider || !isLoopbackBaseUrl(provider.base_url)) return null;
  const slug = provider.preset ?? provider.kind;
  return slug === "ollama" || slug === "lmstudio" ? slug : null;
}
