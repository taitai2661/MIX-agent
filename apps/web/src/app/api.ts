import type { components } from "../generated/api";
import { ja } from "./strings";
import { t } from "./i18n";
import { uuid } from "./uuid";
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
  maxRetries?: number;
  retryDelayMs?: number;
}

export const DEFAULT_MAX_RETRIES = 4;
export const DEFAULT_RETRY_DELAY_MS = 300;
export const MAX_RETRY_DELAY_MS = 5_000;

export function isRetryableStatus(status?: number): boolean {
  if (typeof status !== "number") return false;
  return status === 408 || status === 429 || (status >= 500 && status <= 599);
}

export function getRetryDelay(
  response: Response | undefined,
  attempt: number,
  baseDelayMs: number,
): number {
  if (response?.headers) {
    const retryAfter = response.headers.get("retry-after");
    if (retryAfter) {
      const seconds = Number(retryAfter);
      if (!isNaN(seconds) && seconds > 0) {
        return Math.min(seconds * 1000, 10_000);
      }
    }
  }
  return Math.min(MAX_RETRY_DELAY_MS, baseDelayMs * 2 ** attempt);
}

export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      return reject(new DOMException("Aborted", "AbortError"));
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(new DOMException("Aborted", "AbortError"));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

class HttpError extends Error {
  status?: number;
  constructor(message: string, status?: number) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
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
  if (method === "POST") headers["Idempotency-Key"] = idempotencyKey || uuid();

  const maxRetries = options.maxRetries ?? DEFAULT_MAX_RETRIES;
  const baseDelayMs = options.retryDelayMs ?? DEFAULT_RETRY_DELAY_MS;

  for (let attempt = 0; ; attempt++) {
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
        if (
          attempt < maxRetries &&
          isRetryableStatus(response.status) &&
          !options.signal?.aborted
        ) {
          const delay = getRetryDelay(response, attempt, baseDelayMs);
          cleanup();
          await sleep(delay, options.signal);
          continue;
        }
        const error = await response
          .json()
          .catch(() => ({ detail: t(ja.communicationFailed) }));
        const message =
          typeof error.detail === "string"
            ? t(error.detail)
            : JSON.stringify(error.detail);
        cleanup();
        throw new HttpError(message, response.status);
      }
      cleanup();
      return await response.json();
    } catch (error) {
      cleanup();
      if (options.signal?.aborted) {
        throw new Error(t(ja.requestInterrupted));
      }
      if (timedOut()) {
        throw new Error(t(ja.requestTimeout));
      }
      if (error instanceof HttpError) {
        throw new Error(error.message);
      }
      if (attempt < maxRetries) {
        const delay = getRetryDelay(undefined, attempt, baseDelayMs);
        await sleep(delay, options.signal);
        continue;
      }
      throw error;
    }
  }
}

export async function binary(
  path: string,
  body: FormData | object,
  options: ApiOptions = {},
) {
  const form = body instanceof FormData;
  const maxRetries = options.maxRetries ?? DEFAULT_MAX_RETRIES;
  const baseDelayMs = options.retryDelayMs ?? DEFAULT_RETRY_DELAY_MS;

  for (let attempt = 0; ; attempt++) {
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
        if (
          attempt < maxRetries &&
          isRetryableStatus(r.status) &&
          !options.signal?.aborted
        ) {
          const delay = getRetryDelay(r, attempt, baseDelayMs);
          cleanup();
          await sleep(delay, options.signal);
          continue;
        }
        const e = await r.json().catch(() => ({ detail: ja.operationFailed }));
        const message =
          typeof e.detail === "string" ? e.detail : JSON.stringify(e.detail);
        cleanup();
        throw new HttpError(message, r.status);
      }
      cleanup();
      return r;
    } catch (error) {
      cleanup();
      if (options.signal?.aborted) {
        throw new Error(timedOut() ? ja.requestTimeout : ja.requestInterrupted);
      }
      if (timedOut()) {
        throw new Error(ja.requestTimeout);
      }
      if (error instanceof HttpError) {
        throw new Error(error.message);
      }
      if (attempt < maxRetries) {
        const delay = getRetryDelay(undefined, attempt, baseDelayMs);
        await sleep(delay, options.signal);
        continue;
      }
      throw error;
    }
  }
}
