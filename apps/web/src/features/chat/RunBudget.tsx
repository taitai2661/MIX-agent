import { t } from "@/app/i18n";
import { useEffect, useState } from "react";

import type { Run } from "./types";

/**
 * Remaining seconds shown between two server reads. The server only computes
 * `remaining.max_seconds` when the Run is fetched, so discount the wall-clock
 * time since that fetch to keep the counter live. Once the Run stops running
 * (or is paused for manual browser control) the last server value is kept.
 */
export function remainingSeconds(
  remaining: number | null | undefined,
  active: boolean,
  paused: boolean,
  now: number,
  fetchedAt: number,
): number | null {
  if (remaining == null) return null;
  if (!active || paused) return remaining;
  const elapsed = Math.max(0, Math.floor((now - fetchedAt) / 1000));
  return Math.max(0, remaining - elapsed);
}

export function RunBudget({
  run,
  active,
  fetchedAt,
}: {
  run: Run;
  active: boolean;
  fetchedAt: number;
}) {
  const paused = Boolean(run.browser_manual_active);
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!active || paused) return;
    const timer = window.setInterval(() => setTick((n) => n + 1), 1000);
    return () => window.clearInterval(timer);
  }, [active, paused]);
  const seconds = remainingSeconds(
    run.remaining.max_seconds,
    active,
    paused,
    Date.now(),
    fetchedAt,
  );
  return (
    <p className="run-budget" role="status">
      {run.mode}
      {t(" · 残り ")}
      {seconds ?? 0}
      {t("秒")}
      {" · "}
      {run.remaining.max_steps ?? 0}
      {t("ステップ")}
      {" · "}
      {run.remaining.max_tool_calls ?? 0} Tool Call
    </p>
  );
}
