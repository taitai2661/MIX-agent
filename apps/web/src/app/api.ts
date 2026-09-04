import type { components } from "../generated/api";
export type Row = Omit<components["schemas"]["RecordView"], "data"> & {
  data: Record<string, any>;
};
let csrf = "";
export function setCSRF(value: string) {
  csrf = value;
}
export async function api<T = any>(
  path: string,
  method = "GET",
  body?: unknown,
  idempotencyKey?: string,
): Promise<T> {
  const form = body instanceof FormData;
  const headers: Record<string, string> = { "x-csrf-token": csrf };
  if (body && !form) headers["Content-Type"] = "application/json";
  if (method === "POST")
    headers["Idempotency-Key"] = idempotencyKey || crypto.randomUUID();
  const response = await fetch("/api/v1" + path, {
    method,
    credentials: "same-origin",
    headers,
    body: body ? (form ? body : JSON.stringify(body)) : undefined,
  });
  if (!response.ok) {
    const error = await response
      .json()
      .catch(() => ({ detail: "通信に失敗しました" }));
    throw new Error(
      typeof error.detail === "string"
        ? error.detail
        : JSON.stringify(error.detail),
    );
  }
  return response.json();
}
export async function binary(path: string, body: FormData | object) {
  const form = body instanceof FormData;
  const r = await fetch("/api/v1" + path, {
    method: "POST",
    credentials: "same-origin",
    headers: {
      "x-csrf-token": csrf,
      ...(!form ? { "Content-Type": "application/json" } : {}),
    },
    body: form ? body : JSON.stringify(body),
  });
  if (!r.ok) {
    const e = await r.json().catch(() => ({ detail: "処理に失敗しました" }));
    throw new Error(
      typeof e.detail === "string" ? e.detail : JSON.stringify(e.detail),
    );
  }
  return r;
}
