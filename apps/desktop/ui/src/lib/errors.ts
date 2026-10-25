/** Normalise anything thrown across the Tauri boundary into a message.
 *
 * Tauri rejects a failed `invoke` with the bare `String` the command returned,
 * not with an `Error`, so `err.message` is `undefined` — which is how a
 * provider-creation failure ended up rendering as nothing at all.
 */
export function errorMessage(error: unknown): string {
  if (error == null) return "";
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.length > 0) return message;
  }
  return String(error);
}
