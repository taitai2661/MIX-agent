import { getLanguage, t } from "@/app/i18n";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  CircleDot,
  Clock3,
  ExternalLink,
  HelpCircle,
  Loader2,
  RotateCcw,
  ShieldCheck,
  Wrench,
  XCircle,
} from "lucide-react";
import type { ApprovalDecision, ToolCallHistory } from "./types";
import { ActivityIcon } from "./ToolActivity";

export type { ToolCallHistory } from "./types";

const presentation = {
  completed: { label: "成功", Icon: CheckCircle2, tone: "ok" },
  failed: { label: "失敗", Icon: XCircle, tone: "error" },
  running: { label: "実行中", Icon: CircleDot, tone: "live" },
  waiting_approval: { label: "承認待ち", Icon: Clock3, tone: "wait" },
  unknown: { label: "結果不明", Icon: HelpCircle, tone: "warn" },
} as const;

function formatTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(getLanguage() === "en" ? "en-US" : "ja-JP", {
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

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
  return (
    <li className={"run-step " + call.status}>
      <span className="run-step-icon"><ActivityIcon icon={iconOf(call.activity)} size={17} /></span>
      <div className="run-step-body">
        <div className="run-step-head">
          <b>{call.activity?.label || call.tool_name}</b>
          <span className={"run-step-status " + state.tone}><StateIcon size={13} />{t(state.label)}</span>
          <time dateTime={call.created_at}>{formatTime(call.created_at)}</time>
        </div>
        {call.activity?.detail && <p className="run-step-detail">{call.activity.detail}</p>}
        {call.result_activity?.label && <p className="run-step-detail">{call.result_activity.label}</p>}
        {call.failure && <p className="run-step-failure"><AlertTriangle size={14} /> {call.failure}</p>}
        {sources.length > 0 && (
          <div className="run-step-sources">
            {sources.map((source) => (
              <a key={source.url} href={source.url} target="_blank" rel="noreferrer"><ExternalLink size={12} />{source.host}</a>
            ))}
            {!!call.result_activity?.remaining && <span>{t("あと")} {call.result_activity.remaining} {t("件")}</span>}
          </div>
        )}
        {call.artifact && <a className="run-step-artifact" href={"/api/v1/artifacts/" + call.artifact.artifact_id}>{t("成果物をダウンロード")}</a>}
        {call.retry.available && call.retry.label && <p className="run-step-retry"><RotateCcw size={13} /> {call.retry.label}</p>}
        {hasResult && (
          <details className="run-step-result">
            <summary>{t("結果の詳細")}</summary>
            <pre>{JSON.stringify(call.result, null, 2)}</pre>
          </details>
        )}
        {call.approval && (
          <div className="run-step-approval"><ShieldCheck size={15} /> {t("承認が必要です")}
            <span>
              <button onClick={() => onApproval(call.approval!.id, "once")}>{t("今回のみ")}</button>
              <button onClick={() => onApproval(call.approval!.id, "always")}>{t("常に許可")}</button>
              <button onClick={() => onApproval(call.approval!.id, "denied")}>{t("拒否")}</button>
            </span>
          </div>
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
  if (!calls.length && !running) return null;
  const active = calls.find((call) => call.status === "running" || call.status === "waiting_approval");
  const failed = calls.some((call) => call.status === "failed" || call.status === "unknown");
  const sources = calls.reduce((total, call) => total + sourceCount(call), 0);
  const open = running || failed || calls.some((call) => call.status === "waiting_approval");
  const head = running && active ? active.activity?.label || active.tool_name : t("実行履歴");
  const meta = calls.length
    ? [`${calls.length}${t("件のツール")}`, ...(sources ? [`${sources}${t("件のソース")}`] : [])].join(" · ")
    : t("実行を準備しています…");
  return (
    <section
      className={"run-activity" + (running ? " is-running" : "") + (failed ? " has-error" : "")}
      aria-label={t("ツール実行履歴")}
    >
      <details className="run-activity-details" open={open}>
        <summary className="run-activity-summary">
          <span className="run-activity-icon">
            {running ? <Loader2 className="spin" size={15} /> : failed ? <AlertTriangle size={15} /> : <Wrench size={15} />}
          </span>
          <span className="run-activity-head">
            <b>{head}</b>
            <small>{meta}</small>
          </span>
          {calls.some((call) => call.status === "waiting_approval") && <span className="run-activity-badge">{t("承認待ち")}</span>}
          <ChevronRight className="run-activity-chevron" size={16} />
        </summary>
        <ol className="run-timeline">
          {calls.map((call) => <RunActivityCall key={call.id} call={call} onApproval={onApproval} />)}
        </ol>
      </details>
    </section>
  );
}
