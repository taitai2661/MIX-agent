/// Small display formatters shared across panes.

import { getLanguage, t, tf } from "./i18n";

/** "たった今" / "3分前" / "昨日" / "1月2日" style relative label. */
export function relativeTime(iso: string, now: Date = new Date()): string {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "";
  const diffMs = now.getTime() - then.getTime();
  // Future timestamps (clock skew, bad data) and sub-minute diffs read as now.
  if (!Number.isFinite(diffMs) || diffMs < 60_000) return t("たった今");
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 60) return tf("{n}分前", { n: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return tf("{n}時間前", { n: hours });
  const days = Math.floor(hours / 24);
  if (days === 1) return t("昨日");
  if (days < 7) return tf("{n}日前", { n: days });
  // `undefined` would follow the OS locale, which can disagree with the UI
  // language the user picked in Settings — derive it from `getLanguage()`.
  const locale = getLanguage() === "en" ? "en-US" : "ja-JP";
  return then.toLocaleDateString(locale, { month: "short", day: "numeric" });
}

/** Sidebar grouping bucket for a timestamp. */
export type ThreadGroup = "today" | "yesterday" | "week" | "older";

export function threadGroup(iso: string, now: Date = new Date()): ThreadGroup {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "older";
  // Calendar-aware boundaries (not fixed 24h offsets) so DST transitions
  // do not shift "yesterday"/"week" by an hour.
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfYesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).getTime();
  const startOfWeek = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 6).getTime();
  const time = then.getTime();
  if (time >= startOfToday) return "today";
  if (time >= startOfYesterday) return "yesterday";
  if (time >= startOfWeek) return "week";
  return "older";
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "0ms";
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return seconds ? `${minutes}m ${seconds}s` : `${minutes}m`;
}

/** Short id label for runs/tools ("a1b2c3d4"). */
export function shortId(id: string, len = 8): string {
  return id.slice(0, len);
}

/** Context window as a compact token count: 128000 → "128K", 1000000 → "1M".
 *
 * These numbers sit in dense model rows, where the raw `128,000` is four extra
 * characters of visual noise for information the reader only needs to the
 * nearest power of a thousand. An unknown/zero window renders as "—" rather
 * than a misleading "0". */
export function formatContext(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) return "—";
  const trim = (value: number) => value.toFixed(1).replace(/\.0$/, "");
  if (tokens >= 1_000_000) return `${trim(tokens / 1_000_000)}M`;
  if (tokens >= 1000) return `${trim(tokens / 1000)}K`;
  return String(tokens);
}

/** Model ids are stored as `<provider_id>:<model>` where `provider_id` is a
 * UUID, e.g. `a1830932-37fa-445c-a824-2336c7e2e6b8:liquid/lfm2-1.2b`.
 * Showing the raw id leaks the UUID into the UI. Strip the first segment so
 * the user only sees the model name. Split on the *first* colon so Ollama tags
 * like `llama3.1:8b` stay intact (`<uuid>:llama3.1:8b` → `llama3.1:8b`). */
export function shortModelId(id: string | null | undefined): string {
  if (!id) return "";
  const colon = id.indexOf(":");
  return colon >= 0 ? id.slice(colon + 1) : id;
}

/** Human-readable model name: the registered label when known, otherwise the
 * UUID-stripped model name, otherwise the assistant fallback. */
export function displayModelName(
  id: string | null | undefined,
  models?: ReadonlyArray<{ id: string; label: string }> | undefined,
  fallback = "assistant",
): string {
  if (!id) return fallback;
  const found = models?.find((m) => m.id === id);
  if (found?.label) return found.label;
  const short = shortModelId(id);
  return short || fallback;
}

/** Display-only fallback for leaked DeepSeek DSML tool-call markup.
///
/// The Rust engine heals leaked `<｜DSML｜…>` tags into real tool calls and
/// strips them from the stored transcript. This covers what that cannot:
/// rows persisted before the fix, and any malformed fragment the healer
/// deliberately left untouched (fail-closed). It never executes anything —
/// it only hides raw tags so they cannot reach the transcript as text.
///
/// Handles both the fullwidth (`｜`, U+FF5C) and ASCII (`|`) delimiters,
/// single and doubled, plus the sentence-framing tokens that leak alongside.
 */
export function stripLeakedMarkup(text: string | null | undefined): string {
  if (!text) return "";
  let out = text;
  // Whole invoke blocks first (non-greedy, both bar styles).
  for (const pattern of [
    /<｜｜DSML｜｜invoke[\s\S]*?<\/｜｜DSML｜｜invoke>/g,
    /<｜DSML｜invoke[\s\S]*?<\/｜DSML｜invoke>/g,
    /<\|\|DSML\|\|invoke[\s\S]*?<\/\|\|DSML\|\|invoke>/g,
    /<\|DSML\|invoke[\s\S]*?<\/\|DSML\|invoke>/g,
  ]) {
    out = out.replace(pattern, "");
  }
  // Envelopes and any remaining DSML-looking tag.
  for (const pattern of [
    /<｜｜DSML｜｜tool_calls>/g,
    /<\/｜｜DSML｜｜tool_calls>/g,
    /<｜DSML｜tool_calls>/g,
    /<\/｜DSML｜tool_calls>/g,
    /<\|\|DSML\|\|tool_calls>/g,
    /<\/\|\|DSML\|\|tool_calls>/g,
    /<\|DSML\|tool_calls>/g,
    /<\/\|DSML\|tool_calls>/g,
    /<｜*[｜|]*DSML[｜|]*[^>]*>/g,
    /<\/｜*[｜|]*DSML[｜|]*[^>]*>/g,
    /<\|\|?DSML\|\|?[^>]*>/g,
    /<\/\|\|?DSML\|\|?[^>]*>/g,
  ]) {
    out = out.replace(pattern, "");
  }
  for (const token of [
    "<｜begin▁of▁sentence｜>",
    "<｜end▁of▁sentence｜>",
    "<｜User｜>",
    "<｜Assistant｜>",
    "<|begin_of_sentence|>",
    "<|end_of_sentence|>",
    "<|User|>",
    "<|Assistant|>",
  ]) {
    out = out.split(token).join("");
  }
  return out;
}
