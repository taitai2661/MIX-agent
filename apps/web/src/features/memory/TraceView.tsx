import { useState } from "react";
import { t } from "@/app/i18n";
import { GitBranch, Network, Timer, X } from "lucide-react";

export type TraceHit = {
  id: string;
  content?: string;
  gist?: string;
  relevance?: number;
  selection_reason?: string;
  score_breakdown?: Record<string, number>;
};

export type TraceStep = {
  stage: string;
  depth?: number;
  found?: number;
  admitted?: number;
  rejected?: Array<{ id: string; score: number; threshold: number }>;
  thresholds?: Record<string, number>;
  edges_considered?: number;
  edges_below_threshold?: number;
  accepted?: number;
  frontier_after?: number;
  elapsed_ms?: number;
  considered?: number;
  kept?: number;
  result_limit?: number;
};

export type TraceDebug = {
  elapsed_ms?: number;
  stop_reason?: string;
  // §5 search debug: the ranking that actually ran, echoed verbatim
  // ("recent" / "lexical" / "hybrid") — never re-labelled.
  ranking_mode?: string;
  budget_exhausted?: boolean;
  auto?: Record<string, number>;
  trace?: TraceStep[];
  honored_settings?: string[];
  ignored_settings?: string[];
  seed_ids?: string[];
  association_expansion?: Array<{ source: string; target: string; activation: number; kind?: string }>;
  association_expansion_count?: number;
  association_cutoff?: string | null;
  // Role-based recall() payload shape (tolerated when present).
  considered?: number;
  kept?: number;
  task_keywords?: string[];
  weights?: Record<string, number>;
  scopes?: string[];
  roles?: string[] | null;
  lifecycles?: string[];
};

export type TraceProbe = {
  memories: TraceHit[];
  debug?: TraceDebug | null;
  query?: string;
};

const STOP_LABEL: Record<string, string> = {
  depth: "最大hop到達",
  no_frontier: "探索終了",
  deadline: "検索予算超過",
  candidate_budget: "候補上限",
  no_edges: "関連エッジなし",
};

const SCORE_LABEL: Record<string, string> = {
  lexical: "語彙",
  feature: "概念",
  scope: "スコープ",
  recency: "新しさ",
  association: "連想",
};

function ScoreBars({ breakdown }: { breakdown: Record<string, number> }) {
  const entries = Object.entries(breakdown).filter(([, v]) => typeof v === "number");
  if (!entries.length) return null;
  return (
    <div className="trace-bars" aria-label={t("スコア内訳")}>
      {entries.map(([name, value]) => (
        <div className="trace-bar-row" key={name}>
          <span className="trace-bar-name">{SCORE_LABEL[name] || name}</span>
          <span className="trace-bar" role="img" aria-label={`${name} ${value}`}>
            <span className="trace-bar-fill" style={{ width: `${Math.round(Math.max(0, Math.min(1, value)) * 100)}%` }} />
          </span>
          <span className="trace-bar-value">{value.toFixed(2)}</span>
        </div>
      ))}
    </div>
  );
}

function StepCard({ step, index }: { step: TraceStep; index: number }) {
  const title =
    step.stage === "seed" ? `seed · ${t("候補")} ${step.found ?? 0} / ${t("採用")} ${step.admitted ?? 0}`
    : step.stage === "hop" ? `hop ${step.depth ?? index} · ${t("展開")} ${step.accepted ?? 0}`
    : step.stage === "rank" ? `rank · ${t("採用")} ${step.kept ?? 0} / ${t("候補")} ${step.considered ?? 0}`
    : step.stage;
  return (
    <li className="memory-trace-step">
      <b>{title}</b>
      {step.stage === "seed" && step.thresholds && (
        <span>
          {t("しきい値")}: {Object.entries(step.thresholds).map(([k, v]) => `${k} ${v}`).join(" · ")}
        </span>
      )}
      {step.stage === "seed" && step.rejected && step.rejected.length > 0 && (
        <span>
          {t("却下")} {step.rejected.length} ({t("関連度")}: {step.rejected.slice(0, 3).map((r) => r.score.toFixed(2)).join(", ")}
          {step.rejected.length > 3 ? "…" : ""})
        </span>
      )}
      {step.stage === "hop" && (
        <span>
          {t("考慮エッジ")} {step.edges_considered ?? 0} · {t("閾値未満")} {step.edges_below_threshold ?? 0} · {t("展開")}{" "}
          {step.accepted ?? 0} · frontier {step.frontier_after ?? 0}
          {typeof step.elapsed_ms === "number" ? ` · ${step.elapsed_ms}ms` : ""}
        </span>
      )}
      {step.stage === "rank" && typeof step.result_limit === "number" && <span>limit {step.result_limit}</span>}
    </li>
  );
}

export function TraceView({
  probe,
  query,
  onSelect,
  onClose,
}: {
  probe: TraceProbe;
  query?: string;
  onSelect?: (hit: TraceHit) => void;
  onClose?: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const debug = probe.debug || {};
  const memories = probe.memories || [];
  const steps = debug.trace || [];
  const rank = steps.find((s) => s.stage === "rank");
  const considered = debug.considered ?? rank?.considered ?? steps.find((s) => s.stage === "seed")?.found;
  const kept = debug.kept ?? rank?.kept ?? memories.length;
  const stopKey = debug.stop_reason || "";
  const expansions = debug.association_expansion || [];
  const ignored = debug.ignored_settings || [];
  // §4: rows a link pulled in carry `selection_reason="リンクから"` and are
  // appended after the seeds. Counting them here keeps that contract visible
  // instead of relying on the reader to know why two rows look different.
  const linkHits = memories.filter((hit) => hit.selection_reason === "リンクから");

  const copyJson = async () => {
    try {
      await navigator.clipboard.writeText(JSON.stringify(probe, null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  };

  return (
    <section className="card memory-trace-result" aria-label={t("想起トレース")}>
      <header className="trace-header">
        <h3>
          <Network size={15} /> {t("想起トレース")}
        </h3>
        <span className="trace-actions">
          {(query || probe.query) && <span className="trace-query">{query || probe.query}</span>}
          <button className="trace-copy" onClick={copyJson} type="button">
            {copied ? t("コピーしました") : t("JSONをコピー")}
          </button>
          {onClose && (
            <button className="trace-copy" onClick={onClose} type="button" aria-label={t("トレースを閉じる")}>
              <X size={13} /> {t("閉じる")}
            </button>
          )}
        </span>
      </header>

      <div className="memory-metrics">
        {typeof considered === "number" && <span>{t("候補")} {considered}</span>}
        {typeof kept === "number" && <span>{t("採用")} {kept}</span>}
        {typeof debug.elapsed_ms === "number" && (
          <span>
            <Timer size={11} /> {t("経過")} {debug.elapsed_ms}ms
          </span>
        )}
        {stopKey && <span className="trace-stop">{t(STOP_LABEL[stopKey] || "停止理由")}: {t(STOP_LABEL[stopKey] || stopKey)}</span>}
        {debug.ranking_mode && <span>{t("ランキング方式")}: {debug.ranking_mode}</span>}
        {typeof debug.association_expansion_count === "number" && (
          <span>
            {t("関連展開")} {debug.association_expansion_count}
          </span>
        )}
        {debug.association_cutoff && <span className="trace-warn">{t("打ち切り")}: {debug.association_cutoff}</span>}
        {linkHits.length > 0 && (
          <span>
            seed {memories.length - linkHits.length} · {t("リンクから")} {linkHits.length}
          </span>
        )}
        {debug.budget_exhausted && <span className="trace-warn">{t("予算を使い切りました")}</span>}
        {typeof debug.auto?.trace_count === "number" && <span>{t("記憶")} {debug.auto.trace_count}</span>}
        {typeof debug.auto?.edge_count === "number" && <span>{t("関連エッジ")} {debug.auto.edge_count}</span>}
      </div>

      {ignored.length > 0 && (
        <p className="trace-note">
          {t("指定された上書きは無視されました")}: {ignored.join(", ")}
        </p>
      )}

      {debug.auto && (
        <details className="trace-details">
          <summary>{t("自動設定")}</summary>
          <div className="memory-metrics">
            {Object.entries(debug.auto).map(([k, v]) => (
              <span key={k}>{k} {v}</span>
            ))}
          </div>
        </details>
      )}

      {debug.task_keywords && debug.task_keywords.length > 0 && (
        <p className="trace-note">keywords: {debug.task_keywords.join(", ")}</p>
      )}

      {steps.length > 0 && (
        <ol className="memory-trace-steps" aria-label={t("想起トレース")}>
          {steps.map((step, i) => (
            <StepCard key={`${step.stage}-${i}`} step={step} index={i} />
          ))}
        </ol>
      )}

      {memories.length === 0 ? (
        <p className="trace-empty">{t("該当するTraceはありません。")}</p>
      ) : (
        <div className="trace-hits">
          {memories.map((hit) => (
            <article
              key={hit.id}
              className={hit.selection_reason === "リンクから" ? "memory-trace-hit is-from-link" : "memory-trace-hit"}
              onClick={onSelect ? () => onSelect(hit) : undefined}
              role={onSelect ? "button" : undefined}
              tabIndex={onSelect ? 0 : undefined}
              onKeyDown={onSelect ? (e) => { if (e.key === "Enter") onSelect(hit); } : undefined}
            >
              <header className="trace-hit-head">
                <b>{hit.gist || hit.content?.slice(0, 80) || hit.id.slice(0, 8)}</b>
                {typeof hit.relevance === "number" && (
                  <span className="trace-relevance">{t("関連度")} {hit.relevance.toFixed(2)}</span>
                )}
              </header>
              {hit.content && hit.gist && hit.content !== hit.gist && <p>{hit.content.slice(0, 240)}</p>}
              {!hit.gist && hit.content && <p>{hit.content.slice(0, 240)}</p>}
              {hit.selection_reason && <p className="memory-selection-reason">{hit.selection_reason}</p>}
              {hit.score_breakdown && <ScoreBars breakdown={hit.score_breakdown} />}
            </article>
          ))}
        </div>
      )}

      {expansions.length > 0 && (
        <details className="trace-details">
          <summary>
            <GitBranch size={13} /> {t("関連展開")} ({expansions.length})
          </summary>
          <ul className="trace-expansion">
            {expansions.slice(0, 20).map((e, i) => (
              <li key={i}>
                <code>{e.source.slice(0, 8)}</code> → <code>{e.target.slice(0, 8)}</code>
                {e.kind ? (
                  <>
                    {" · "}
                    <code>{e.kind}</code>
                  </>
                ) : null}{" "}
                · {e.activation.toFixed(2)}
              </li>
            ))}
            {expansions.length > 20 && <li>… +{expansions.length - 20}</li>}
          </ul>
        </details>
      )}
    </section>
  );
}
