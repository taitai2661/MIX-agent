import { AlertTriangle, CheckCircle2, CircleDot, Clock3, ExternalLink, HelpCircle, RotateCcw, ShieldCheck, Wrench, XCircle } from "lucide-react";
import type { ApprovalDecision, ToolCallHistory } from "./types";

export type { ToolCallHistory } from "./types";

const presentation = {
  completed: { label: "成功", Icon: CheckCircle2 },
  failed: { label: "失敗", Icon: XCircle },
  running: { label: "実行中", Icon: CircleDot },
  waiting_approval: { label: "承認待ち", Icon: Clock3 },
  unknown: { label: "結果不明", Icon: HelpCircle },
} as const;

export function ToolHistory({ calls, onApproval }: { calls: ToolCallHistory[]; onApproval: (id: string, decision: ApprovalDecision) => void }) {
  if (!calls.length) return null;
  return (
    <section className="tool-history" aria-label="ツール実行履歴">
      <div className="tool-history-heading"><Wrench size={15} /> 実行履歴 <span>{calls.length}件</span></div>
      {calls.map((call) => {
        const state = presentation[call.status];
        const StateIcon = state.Icon;
        const sourceCount = call.result_activity?.sources?.length || 0;
        const hasDetails = !!(call.activity?.detail || sourceCount || call.result_activity?.remaining || call.artifact || call.retry.label || call.result !== undefined);
        return <article className={'tool-history-card ' + call.status} key={call.id}>
          <details className="tool-history-details" open={call.status === "waiting_approval" || call.status === "failed"}>
            <summary>
              <span className="tool-history-status"><StateIcon size={15} />{state.label}</span>
              <b>{call.activity?.label || call.tool_name}</b>
              {(sourceCount > 0 || call.artifact) && <span className="tool-history-badges">{sourceCount > 0 && `${sourceCount}件のソース`}{call.artifact && "成果物"}</span>}
              <time dateTime={call.created_at}>{new Intl.DateTimeFormat("ja-JP", { hour: "2-digit", minute: "2-digit" }).format(new Date(call.created_at))}</time>
            </summary>
            {hasDetails && <div className="tool-history-detail-content">
              {call.activity?.detail && <p className="tool-history-detail">{call.activity.detail}</p>}
              {call.result_activity?.sources?.length ? <div className="tool-history-sources">
                {call.result_activity.sources.map((source) => <a key={source.url} href={source.url} target="_blank" rel="noreferrer"><ExternalLink size={12} />{source.host}</a>)}
                {!!call.result_activity.remaining && <span>あと {call.result_activity.remaining} 件</span>}
              </div> : null}
              {call.artifact && <a className="tool-history-artifact" href={'/api/v1/artifacts/' + call.artifact.artifact_id}>成果物をダウンロード</a>}
              <p className="tool-history-retry"><RotateCcw size={13} /> {call.retry.label}</p>
              {call.result !== undefined && call.result !== null && <div className="tool-history-result"><span>結果の詳細</span><pre>{JSON.stringify(call.result, null, 2)}</pre></div>}
            </div>}
          </details>
          {call.failure && <p className="tool-history-failure"><AlertTriangle size={14} /> {call.failure}</p>}
          {call.approval && <div className="tool-history-approval"><ShieldCheck size={15} /> 承認が必要です
            <span><button onClick={() => onApproval(call.approval!.id, "once")}>今回のみ</button><button onClick={() => onApproval(call.approval!.id, "always")}>常に許可</button><button onClick={() => onApproval(call.approval!.id, "denied")}>拒否</button></span>
          </div>}
        </article>;
      })}
    </section>
  );
}
