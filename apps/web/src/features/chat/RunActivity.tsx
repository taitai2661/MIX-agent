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
import type { Approval, ApprovalDecision, ToolCallHistory } from "./types";
import { useRef, useState } from "react";
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

function textValue(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function ApprovalPayload({ data }: { data: Approval["data"] }) {
  const args = data.arguments || {};
  const command = textValue(args.command);
  const path = textValue(args.path);
  // The edit tool names its halves `old_string` / `new_string`; a preview that
  // only looked for `old` / `new` showed a bare path for every edit approval.
  const before = stringValue(args.old ?? args.old_string);
  const after = stringValue(args.new ?? args.new_string);
  const content = stringValue(args.content);
  const url = textValue(args.url);
  const dedicated = new Set([
    "command",
    "path",
    "old",
    "new",
    "old_string",
    "new_string",
    "content",
    "url",
  ]);
  const remaining = Object.entries(args).filter(
    ([key, value]) => !dedicated.has(key) && value !== null && value !== undefined,
  );
  const scope = Object.entries(data.scope || {}).filter(
    ([, value]) => value !== null && value !== undefined && value !== "",
  );
  if (!data.tool && before === undefined && after === undefined && content === undefined && !command && !path && !url && !remaining.length && !scope.length) {
    return null;
  }
  return (
    <div className="run-step-approval-payload">
      {data.tool && (
        <p className="run-step-approval-tool">
          <span>{t("ツール")}</span>
          <code>{data.tool}</code>
          {data.risk && <em className="run-step-approval-risk">{data.risk}</em>}
        </p>
      )}
      {path && (
        <p className="run-step-approval-field">
          <span>{t("パス")}</span>
          <code>{path}</code>
        </p>
      )}
      {command && <pre className="run-step-approval-code">{command}</pre>}
      {url && (
        <p className="run-step-approval-field">
          <span>URL</span>
          <code>{url}</code>
        </p>
      )}
      {(before !== undefined || after !== undefined) && (
        <div className="run-step-approval-diff">
          {before !== undefined && (
            <div>
              <b>{t("変更前")}</b>
              <pre>{before}</pre>
            </div>
          )}
          {after !== undefined && (
            <div>
              <b>{t("変更後")}</b>
              <pre>{after}</pre>
            </div>
          )}
        </div>
      )}
      {content !== undefined && (
        <div className="run-step-approval-diff">
          <div>
            <b>{t("内容")}</b>
            <pre>{content}</pre>
          </div>
        </div>
      )}
      {remaining.map(([key, value]) => (
        <p className="run-step-approval-field" key={key}>
          <span>{key}</span>
          <code>{typeof value === "string" ? value : JSON.stringify(value)}</code>
        </p>
      ))}
      {scope.length > 0 && (
        <div className="run-step-approval-scope">
          <b>{t("常に許可の範囲")}</b>
          <dl>
            {scope.map(([key, value]) => (
              <div key={key}>
                <dt>{key}</dt>
                <dd>{typeof value === "string" ? value : JSON.stringify(value)}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}
    </div>
  );
}

function RunActivityCall({ call, approvals, onApproval }: { call: ToolCallHistory; approvals?: Approval[]; onApproval: (id: string, decision: ApprovalDecision) => void | Promise<void> }) {
  const state = presentation[call.status];
  const [pendingDecision, setPendingDecision] = useState<ApprovalDecision | null>(null);
  const submitting = useRef(false);
  const approval = approvals?.find((item) => item.id === call.approval?.id);
  async function decide(decision: ApprovalDecision) {
    if (!call.approval || submitting.current) return;
    submitting.current = true;
    setPendingDecision(decision);
    try {
      await onApproval(call.approval.id, decision);
    } finally {
      submitting.current = false;
      setPendingDecision(null);
    }
  }
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
          <div className="run-step-approval" aria-busy={pendingDecision !== null}>
            <span className="run-step-approval-label"><ShieldCheck size={14} /> {t("承認が必要です")}</span>
            {approval && <ApprovalPayload data={approval.data} />}
            <span className="run-step-approval-actions">
              <button disabled={pendingDecision !== null} onClick={() => void decide("once")}>{t("今回のみ")}</button>
              <button disabled={pendingDecision !== null} onClick={() => void decide("always")}>{t("この操作を常に許可")}</button>
              <button disabled={pendingDecision !== null} onClick={() => void decide("denied")}>{t("拒否")}</button>
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
  approvals,
  running = false,
  onApproval,
}: {
  calls: ToolCallHistory[];
  approvals?: Approval[];
  running?: boolean;
  onApproval: (id: string, decision: ApprovalDecision) => void | Promise<void>;
}) {
  if (!calls.length) return null;
  const active = calls.find((call) => call.status === "running" || call.status === "waiting_approval");
  const failed = calls.some((call) => call.status === "failed" || call.status === "unknown");
  const waiting = calls.some((call) => call.status === "waiting_approval");
  const sources = calls.reduce((total, call) => total + sourceCount(call), 0);
  const open = running || waiting;
  const [userOpen, setUserOpen] = useState<boolean | null>(null);
  const detailsOpen = userOpen ?? open;
  const meta = [`${calls.length}${t("件のツール")}`, ...(sources ? [`${sources}${t("件のソース")}`] : [])].join(" · ");
  const head = running && active ? active.activity?.label || active.tool_name : meta;
  const HeaderIcon = running ? Loader2 : waiting ? Clock3 : failed ? AlertTriangle : CheckCircle2;
  return (
    <section
      className={"run-activity" + (running ? " is-running" : "") + (failed ? " has-error" : "")}
      aria-label={t("ツール実行履歴")}
    >
      <details className="run-activity-details" open={detailsOpen} onToggle={(e) => setUserOpen(e.currentTarget.open)}>
        <summary className="run-activity-summary">
          <span className="run-activity-icon">
            <HeaderIcon className={running ? "spin" : undefined} size={15} />
          </span>
          <span className="run-activity-head">
            <b>{head}</b>
          </span>
          {waiting && <span className="run-activity-badge" style={{ background: "var(--accent-soft)", color: "var(--accent)" }}>{t("承認待ち")}</span>}
          {failed && !running && <span className="run-activity-badge" style={{ background: "#c23b3b18", color: "var(--danger)" }}>{t("エラー")}</span>}
          <ChevronDown className="run-activity-chevron" size={15} />
        </summary>
        <ol className="run-timeline">
          {calls.map((call) => <RunActivityCall key={call.id} call={call} approvals={approvals} onApproval={onApproval} />)}
        </ol>
      </details>
    </section>
  );
}
