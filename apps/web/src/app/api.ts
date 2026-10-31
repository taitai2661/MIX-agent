import type { components } from "../generated/api";
import { ja } from "./strings";
import { getLanguage, t } from "./i18n";
import { errorCodeText } from "./errorCodes";
import { uuid } from "./uuid";
export type Row = Omit<components["schemas"]["RecordView"], "data"> & {
  // Row payloads are per-endpoint JSON objects; the generated RecordView models
  // them as an open map, so the concrete shape is only known at each call site.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic row payload, narrowed per feature
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
  /**
   * Opt in to retrying a mutating request. Only endpoints that honour an
   * Idempotency-Key (send/edit message) may safely do this; everything else
   * would risk creating the same provider, upload, or schedule twice.
   */
  retryUnsafe?: boolean;
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

export class HttpError extends Error {
  status?: number;
  constructor(message: string, status?: number) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
}

type ErrorDetail =
  | Array<{ loc?: unknown[]; msg?: string }>
  | string
  | undefined;
type ApiErrorBody = { detail?: ErrorDetail; code?: string };

/** Decode a successful response without assuming it has a JSON body. */
async function parseSuccess<T>(response: Response): Promise<T> {
  if (response.status === 204) return undefined as T;
  // Test doubles (and a few non-browser runtimes) only implement `json()`.
  if (typeof response.text !== "function") return (await response.json()) as T;
  const text = await response.text();
  if (!text.trim()) return undefined as T;
  return JSON.parse(text) as T;
}

function errorMessage(body: ApiErrorBody, status: number): string {
  const { detail, code } = body;
  // Prefer the stable code: it survives wording changes and does not depend on
  // the server's sentence being present in the i18n table. Japanese keeps the
  // server's canonical message; English uses the code table, then a generic
  // status message, so an untranslated code never leaks Japanese.
  if (getLanguage() === "en") {
    const text = errorCodeText(code) ?? errorCodeText(`common.http_${status}`);
    if (text) return text;
  }
  if (code && typeof detail === "string" && getLanguage() === "ja")
    return detail;
  if (typeof detail === "string") return t(detail);
  if (Array.isArray(detail)) {
    const fields = detail
      .map((item) => {
        if (!item || typeof item !== "object") return "";
        const path = Array.isArray(item.loc) ? item.loc.slice(1).join(".") : "";
        const message = typeof item.msg === "string" ? t(item.msg) : "";
        return [path, message].filter(Boolean).join(": ");
      })
      .filter(Boolean);
    if (fields.length) return fields.join("\n");
  }
  return t(ja.communicationFailed);
}

function canRetry(method: string, options: ApiOptions): boolean {
  const verb = method.toUpperCase();
  return (
    verb === "GET" ||
    verb === "HEAD" ||
    verb === "OPTIONS" ||
    options.retryUnsafe === true
  );
}

function notifyUnauthorized(path: string, status?: number) {
  if (status !== 401 || path.startsWith("/auth/login")) return;
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent("mix:unauthorized"));
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

export async function api<T = unknown>(
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
  const retryable = canRetry(method, options);

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
          retryable &&
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
        cleanup();
        notifyUnauthorized(path, response.status);
        throw new HttpError(
          errorMessage(error, response.status),
          response.status,
        );
      }
      // Keep the timer armed through the body read: a server that sends
      // headers and then stalls the stream used to hang the caller forever.
      const parsed = await parseSuccess<T>(response);
      cleanup();
      return parsed;
    } catch (error) {
      cleanup();
      if (options.signal?.aborted) {
        throw new Error(t(ja.requestInterrupted));
      }
      if (timedOut()) {
        throw new Error(t(ja.requestTimeout));
      }
      if (error instanceof HttpError) {
        throw error;
      }
      if (retryable && attempt < maxRetries) {
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
  const retryable = options.retryUnsafe === true;

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
          retryable &&
          attempt < maxRetries &&
          isRetryableStatus(r.status) &&
          !options.signal?.aborted
        ) {
          const delay = getRetryDelay(r, attempt, baseDelayMs);
          cleanup();
          await sleep(delay, options.signal);
          continue;
        }
        const e = await r
          .json()
          .catch(() => ({ detail: t(ja.operationFailed) }));
        cleanup();
        notifyUnauthorized(path, r.status);
        throw new HttpError(errorMessage(e, r.status), r.status);
      }
      cleanup();
      return r;
    } catch (error) {
      cleanup();
      if (options.signal?.aborted) {
        throw new Error(
          t(timedOut() ? ja.requestTimeout : ja.requestInterrupted),
        );
      }
      if (timedOut()) {
        throw new Error(t(ja.requestTimeout));
      }
      if (error instanceof HttpError) {
        throw error;
      }
      if (retryable && attempt < maxRetries) {
        const delay = getRetryDelay(undefined, attempt, baseDelayMs);
        await sleep(delay, options.signal);
        continue;
      }
      throw error;
    }
  }
}
