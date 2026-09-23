import type { components } from "../generated/api";
import { ja } from "./strings";
export type Row = Omit<components["schemas"]["RecordView"], "data"> & {
  data: Record<string, any>;
};
let csrf = "";
export function setCSRF(value: string) {
  csrf = value;
}

export interface ApiOptions {
  signal?: AbortSignal;
  timeoutMs?: number;
}

const DEFAULT_TIMEOUT_MS = 120_000;
const BINARY_TIMEOUT_MS = 600_000;

function pipeTimeout(
  controller: AbortController,
  timeoutMs: number,
  signal?: AbortSignal,
) {
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeoutMs);
  const onAbort = () => controller.abort(signal?.reason);
  signal?.addEventListener("abort", onAbort, { once: true });
  return {
    timedOut: () => timedOut,
    cleanup: () => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
    },
  };
}

export async function api<T = any>(
  path: string,
  method = "GET",
  body?: unknown,
  idempotencyKey?: string,
  options: ApiOptions = {},
): Promise<T> {
  const form = body instanceof FormData;
  const headers: Record<string, string> = { "x-csrf-token": csrf };
  if (body && !form) headers["Content-Type"] = "application/json";
  if (method === "POST")
    headers["Idempotency-Key"] = idempotencyKey || crypto.randomUUID();
  const controller = new AbortController();
  const { timedOut, cleanup } = pipeTimeout(
    controller,
    options.timeoutMs ?? DEFAULT_TIMEOUT_MS,
    options.signal,
  );
  try {
    const response = await fetch("/api/v1" + path, {
      method,
      credentials: "same-origin",
      headers,
      signal: controller.signal,
      body: body ? (form ? body : JSON.stringify(body)) : undefined,
    });
    if (!response.ok) {
      const error = await response
        .json()
        .catch(() => ({ detail: ja.communicationFailed }));
      throw new Error(
        typeof error.detail === "string"
          ? error.detail
          : JSON.stringify(error.detail),
      );
    }
    return response.json();
  } catch (error) {
    if (controller.signal.aborted)
      throw new Error(timedOut() ? ja.requestTimeout : ja.requestInterrupted);
    throw error;
  } finally {
    cleanup();
  }
}
export async function binary(path: string, body: FormData | object, options: ApiOptions = {}) {
  const form = body instanceof FormData;
  const controller = new AbortController();
  const { timedOut, cleanup } = pipeTimeout(
    controller,
    options.timeoutMs ?? BINARY_TIMEOUT_MS,
    options.signal,
  );
  try {
    const r = await fetch("/api/v1" + path, {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "x-csrf-token": csrf,
        ...(!form ? { "Content-Type": "application/json" } : {}),
      },
      signal: controller.signal,
      body: form ? body : JSON.stringify(body),
    });
    if (!r.ok) {
      const e = await r.json().catch(() => ({ detail: ja.operationFailed }));
      throw new Error(
        typeof e.detail === "string" ? e.detail : JSON.stringify(e.detail),
      );
    }
    return r;
  } catch (error) {
    if (controller.signal.aborted)
      throw new Error(timedOut() ? ja.requestTimeout : ja.requestInterrupted);
    throw error;
  } finally {
    cleanup();
  }
}
