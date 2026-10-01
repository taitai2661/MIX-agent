import { t } from "@/app/i18n";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  Clock3,
  ExternalLink,
  HelpCircle,
  Loader2,
  RotateCcw,
  ShieldCheck,
} from "lucide-react";
import type { ApprovalDecision, ToolCallHistory } from "./types";
import { ActivityIcon } from "./ToolActivity";

export type { ToolCallHistory } from "./types";

const presentation = {
  completed: { label: "成功", Icon: CheckCircle2, tone: "ok" },
  failed: { label: "失敗", Icon: AlertTriangle, tone: "error" },
  running: { label: "実行中", Icon: Loader2, tone: "live" },
  waiting_approval: { label: "承認待ち", Icon: Clock3, tone: "wait" },
  unknown: { label: "結果不明", Icon: HelpCircle, tone: "warn" },
} as const;

function sourceCount(call: ToolCallHistory) {
  return call.result_activity?.sources?.length || 0;
}

function iconOf(activity: ToolCallHistory["activity"]): string | undefined {
  const icon = (activity as { icon?: unknown } | null | undefined)?.icon;
  return typeof icon === "string" ? icon : undefined;
}

function RunActivityCall({ call, onApproval }: { call: ToolCallHistory; onApproval: (id: string, decision: ApprovalDecision) => void }) {
  const state = presentation[call.status];
  const StateIcon = state.Icon;
  const sources = call.result_activity?.sources || [];
  const hasResult = call.result !== undefined && call.result !== null;
  const hasExtra = Boolean(call.result_activity?.label) || sources.length > 0 || Boolean(call.artifact) || hasResult;
  const retryable = Boolean(call.retry.available && call.retry.label);
  return (
    <li className={"run-step " + call.status}>
      <span className="run-step-icon"><ActivityIcon icon={iconOf(call.activity)} size={16} /></span>
      <div className="run-step-body">
        <div className="run-step-head">
          <b>{call.activity?.label || call.tool_name}</b>
          {call.activity?.detail && <span className="run-step-detail">{call.activity.detail}</span>}
          <span className={"run-step-state " + state.tone}>
            <StateIcon size={14} />
            <span className="sr-only">{t(state.label)}</span>
          </span>
        </div>
        {call.failure && <p className="run-step-failure"><AlertTriangle size={13} /> {call.failure}</p>}
        {retryable && <p className="run-step-retry"><RotateCcw size={12} /> {call.retry.label}</p>}
        {call.approval && call.status === "waiting_approval" && (
          <div className="run-step-approval">
            <span className="run-step-approval-label"><ShieldCheck size={14} /> {t("承認が必要です")}</span>
            <span className="run-step-approval-actions">
              <button onClick={() => onApproval(call.approval!.id, "once")}>{t("今回のみ")}</button>
              <button onClick={() => onApproval(call.approval!.id, "always")}>{t("常に許可")}</button>
              <button onClick={() => onApproval(call.approval!.id, "denied")}>{t("拒否")}</button>
            </span>
          </div>
        )}
        {hasExtra && (
          <details className="run-step-result">
            <summary>{t("詳細")}</summary>
            <div className="run-step-result-body">
              {call.result_activity?.label && <p className="run-step-result-label">{call.result_activity.label}</p>}
              {sources.length > 0 && (
                <div className="run-step-sources">
                  {sources.map((source) => (
                    <a key={source.url} href={source.url} target="_blank" rel="noreferrer"><ExternalLink size={12} />{source.host}</a>
                  ))}
                  {!!call.result_activity?.remaining && <span>{t("あと")} {call.result_activity.remaining} {t("件")}</span>}
                </div>
              )}
              {call.artifact && <a className="run-step-artifact" href={"/api/v1/artifacts/" + call.artifact.artifact_id}>{t("成果物をダウンロード")}</a>}
              {hasResult && <pre>{JSON.stringify(call.result, null, 2)}</pre>}
            </div>
          </details>
        )}
      </div>
    </li>
  );
}

export function RunActivity({
  calls,
  running = false,
  onApproval,
}: {
  calls: ToolCallHistory[];
  running?: boolean;
  onApproval: (id: string, decision: ApprovalDecision) => void;
}) {
  if (!calls.length) return null;
  const active = calls.find((call) => call.status === "running" || call.status === "waiting_approval");
  const failed = calls.some((call) => call.status === "failed" || call.status === "unknown");
  const waiting = calls.some((call) => call.status === "waiting_approval");
  const sources = calls.reduce((total, call) => total + sourceCount(call), 0);
  const open = running || waiting;
  const meta = [`${calls.length}${t("件のツール")}`, ...(sources ? [`${sources}${t("件のソース")}`] : [])].join(" · ");
  const head = running && active ? active.activity?.label || active.tool_name : meta;
  const HeaderIcon = running ? Loader2 : waiting ? Clock3 : failed ? AlertTriangle : CheckCircle2;
  return (
    <section
      className={"run-activity" + (running ? " is-running" : "") + (failed ? " has-error" : "")}
      aria-label={t("ツール実行履歴")}
    >
      <details className="run-activity-details" open={open}>
        <summary className="run-activity-summary">
          <span className="run-activity-icon">
            <HeaderIcon className={running ? "spin" : undefined} size={15} />
          </span>
          <span className="run-activity-head">
            <b>{head}</b>
          </span>
          {waiting && <span className="run-activity-badge">{t("承認待ち")}</span>}
          <ChevronDown className="run-activity-chevron" size={15} />
        </summary>
        <ol className="run-timeline">
          {calls.map((call) => <RunActivityCall key={call.id} call={call} onApproval={onApproval} />)}
        </ol>
      </details>
    </section>
  );
}
