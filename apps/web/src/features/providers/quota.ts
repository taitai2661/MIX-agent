import { getLanguage } from "@/app/i18n";

/**
 * Render a quota reset timestamp in the viewer's time zone, with UTC alongside.
 *
 * The provider reports epoch seconds in UTC; the scheduler uses the browser
 * zone, so hard-coding `Asia/Tokyo` made the two screens disagree. `timeZone`
 * is injectable for deterministic tests.
 */
export function resetTimeLabel(
  seconds: number,
  timeZone = localTimeZone(),
): string {
  if (!seconds) return "";
  const at = new Date(seconds * 1000);
  if (Number.isNaN(at.getTime())) return "";
  const locale = getLanguage() === "en" ? "en-US" : "ja-JP";
  let local: string;
  try {
    local = at.toLocaleString(locale, {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  } catch {
    return `${at.toISOString()} (UTC)`;
  }
  const utc = at.toLocaleString("en-GB", {
    timeZone: "UTC",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return `${local} (${timeZone} / UTC ${utc})`;
}

export function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}
