import { tf } from "./i18n";
import { LOCAL_ERRORS } from "./localErrors";

/** Prefix written by `apps/desktop/providers/src/local_error.rs`. */
const LOCAL_ERROR_PREFIX = "mix-local-error:";

/**
 * Translate a coded backend error into the active UI language.
 *
 * Anything that does not start with the prefix is returned untouched: most
 * commands still return plain messages, and a new code the UI does not know
 * (older UI / newer backend) must stay visible rather than disappear.
 */
export function translateBackendError(message: string): string {
  if (!message.startsWith(LOCAL_ERROR_PREFIX)) return message;
  const body = message.slice(LOCAL_ERROR_PREFIX.length);
  const separator = body.indexOf("|");
  if (separator < 0) return message;
  const code = body.slice(0, separator);
  const spec = LOCAL_ERRORS[code];
  if (!spec) return message;
  let args: unknown;
  try {
    args = JSON.parse(body.slice(separator + 1));
  } catch {
    return message;
  }
  const values = Array.isArray(args) ? args : [];
  const params: Record<string, string> = {};
  spec.params.forEach((name, index) => {
    params[name] = String(values[index] ?? "");
  });
  return tf(spec.template, params);
}

/** Normalise anything thrown across the Tauri boundary into a message.
 *
 * Tauri rejects a failed `invoke` with the bare `String` the command returned,
 * not with an `Error`, so `err.message` is `undefined` — which is how a
 * provider-creation failure ended up rendering as nothing at all.
 *
 * Local-runtime failures carry a code the UI knows how to localize; everything
 * else keeps the backend's text.
 */
export function errorMessage(error: unknown): string {
  if (error == null) return "";
  if (typeof error === "string") return translateBackendError(error);
  if (error instanceof Error) return translateBackendError(error.message);
  if (typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.length > 0) {
      return translateBackendError(message);
    }
  }
  return translateBackendError(String(error));
}
