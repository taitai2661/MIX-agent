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

function ratio(used: number | null | undefined, limit: number | null | undefined): number | null {
  if (typeof used !== "number" || typeof limit !== "number" || limit <= 0) return null;
  return Math.max(0, Math.min(1, 1 - used / limit));
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
  const stepsRemaining = run.remaining.max_steps ?? 0;
  const callsRemaining = run.remaining.max_tool_calls ?? 0;
  const stepsTotal = run.budget.max_steps ?? 0;
  const callsTotal = run.budget.max_tool_calls ?? 0;
  const stepsUsed = stepsTotal - stepsRemaining;
  const callsUsed = callsTotal - callsRemaining;
  const stepsRatio = ratio(stepsUsed, stepsTotal);
  const callsRatio = ratio(callsUsed, callsTotal);
  return (
    <div className="run-budget-card" role="status">
      <p className="run-budget">
        {run.mode}
        {t(" · 残り ")}
        {seconds ?? 0}
        {t("秒")}
        {" · "}
        {stepsRemaining}
        {t("ステップ")}
        {" · "}
        {callsRemaining} Tool Call
        {run.budget_extensions_max > 0 ? (
          <span className="run-budget-extension">
            {" · "}
            {t("延長")} {run.budget_extensions_used}/{run.budget_extensions_max}
          </span>
        ) : null}
      </p>
      {(stepsRatio != null || callsRatio != null) && (
        <div className="run-budget-bars" aria-hidden>
          {stepsRatio != null && (
            <div className="run-budget-bar" title={`${stepsRemaining}/${stepsTotal} ステップ`}>
              <span style={{ width: `${Math.round(stepsRatio * 100)}%` }} />
            </div>
          )}
          {callsRatio != null && (
            <div className="run-budget-bar" title={`${callsRemaining}/${callsTotal} Tool Call`}>
              <span style={{ width: `${Math.round(callsRatio * 100)}%` }} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
