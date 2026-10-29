import { t } from "@/app/i18n";
import { Button } from "@/components/button";

import {
  BudgetExtensionPanel,
  CheckpointsPanel,
  StagnationPanel,
} from "./AgentLoop";
import { ChatMarkdown } from "./ChatMarkdown";
import { ReasoningSummary } from "./ModeStatus";
import { RunBudget } from "./RunBudget";
import type { Run, StagnationFinding } from "./types";

export type ChatRunPanelsProps = {
  streamText: string;
  reasoning: string;
  active: boolean;
  run?: Run;
  runId: string;
  fetchedAt: number;
  canRetry: boolean;
  retryDisabled: boolean;
  onRetry: () => void;
  onError: (error: unknown) => void;
  onResumed: () => void;
  onBudgetResolved: () => void;
};

/** Live stream text, run notices/budget, and recovery panels. */
export function ChatRunPanels({
  streamText,
  reasoning,
  active,
  run,
  runId,
  fetchedAt,
  canRetry,
  retryDisabled,
  onRetry,
  onError,
  onResumed,
  onBudgetResolved,
}: ChatRunPanelsProps) {
  return (
    <>
      <ReasoningSummary text={reasoning} />
      {streamText && (
        <article className="message assistant">
          <div className="message-label">
            MIX agent <span className="pulse" />
          </div>
          <div className="markdown">
            <ChatMarkdown>{streamText}</ChatMarkdown>
          </div>
        </article>
      )}
      {active && !streamText && (
        <p className="processing" role="status">
          <span className="pulse" />
          {t(
            run?.status === "paused"
              ? "ブラウザを手動操作中"
              : run?.status === "waiting_approval"
                ? "操作の承認を待っています"
                : "処理しています…",
          )}
        </p>
      )}
      {run?.reason && (
        <div className="notice">
          <span>{run.reason}</span>
          {(run.status === "failed" || run.status === "cancelled") &&
            canRetry && (
              <span className="notice-actions">
                <Button
                  type="button"
                  variant="outline"
                  onClick={onRetry}
                  disabled={retryDisabled}
                >
                  {t("再試行")}
                </Button>
              </span>
            )}
        </div>
      )}
      {run && active && (
        <RunBudget run={run} active={!!active} fetchedAt={fetchedAt} />
      )}
      {run?.status === "interrupted" && (
        <CheckpointsPanel run={run} onError={onError} onResumed={onResumed} />
      )}
      {run?.status === "budget_extension_pending" &&
        run.budget_extension_request && (
          <BudgetExtensionPanel
            runId={runId}
            request={run.budget_extension_request}
            used={run.budget_extensions_used}
            max={run.budget_extensions_max}
            onError={onError}
            onResolved={onBudgetResolved}
          />
        )}
      {run?.stagnation && run.stagnation.length > 0 && (
        <StagnationPanel findings={run.stagnation as StagnationFinding[]} />
      )}
    </>
  );
}
