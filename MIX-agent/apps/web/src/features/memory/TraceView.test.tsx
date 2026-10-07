import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { TraceView, type TraceProbe } from "./TraceView";

const probe: TraceProbe = {
  query: "Postgres migration",
  memories: [
    {
      id: "mem-1",
      content: "Postgres migration では単一トランザクションで流す",
      gist: "Postgres migration の手順",
      relevance: 0.86,
      selection_reason: "現在の話題と強く一致",
      score_breakdown: { lexical: 0.75, feature: 0.42, scope: 1, recency: 0.9, association: 0 },
    },
    {
      id: "mem-2",
      content: "失敗したマイグレーションは手動で戻す",
      relevance: 0.41,
      selection_reason: "関連する記憶から連想",
      score_breakdown: { lexical: 0.2, feature: 0.35, scope: 0.82, recency: 0.5, association: 0.3 },
    },
  ],
  debug: {
    elapsed_ms: 12.5,
    stop_reason: "candidate_budget",
    budget_exhausted: false,
    auto: { max_depth: 1, max_candidates: 32, retrieval_budget_ms: 82, min_association_weight: 0.2, activation_decay: 0.6, trace_count: 10, edge_count: 4 },
    trace: [
      { stage: "seed", found: 4, admitted: 2, rejected: [{ id: "mem-x", score: 0.1, threshold: 0.18 }], thresholds: { established: 0.18, latent: 0.32 } },
      { stage: "hop", depth: 1, edges_considered: 4, edges_below_threshold: 2, accepted: 1, frontier_after: 1, elapsed_ms: 9.1 },
      { stage: "rank", considered: 3, kept: 2, result_limit: 8 },
    ],
    honored_settings: [],
    ignored_settings: ["max_depth"],
    seed_ids: ["mem-1", "mem-2"],
    association_expansion: [{ source: "mem-1", target: "mem-2", activation: 0.3 }],
  },
};

describe("TraceView", () => {
  it("renders metrics, pipeline steps, and per-hit score breakdowns", () => {
    const html = renderToStaticMarkup(<TraceView probe={probe} />);
    expect(html).toContain("想起トレース");
    expect(html).toContain("Postgres migration");
    expect(html).toContain("候補");
    expect(html).toContain("12.5ms");
    expect(html).toContain("候補上限");
    expect(html).toContain("seed");
    expect(html).toContain("hop 1");
    expect(html).toContain("rank");
    expect(html).toContain("関連度 0.86");
    expect(html).toContain("スコア内訳");
    expect(html).toContain("0.75");
    expect(html).toContain("関連展開");
    expect(html).toContain("指定された上書きは無視されました");
    expect(html).toContain("自動設定");
  });

  it("warns when the retrieval budget is exhausted", () => {
    const html = renderToStaticMarkup(
      <TraceView probe={{ memories: [], debug: { elapsed_ms: 400, stop_reason: "deadline", budget_exhausted: true, trace: [] } }} />,
    );
    expect(html).toContain("予算を使い切りました");
    expect(html).toContain("検索予算超過");
    expect(html).toContain("該当するTraceはありません。");
  });

  it("renders without debug payload", () => {
    const html = renderToStaticMarkup(<TraceView probe={{ memories: [] }} />);
    expect(html).toContain("想起トレース");
    expect(html).toContain("該当するTraceはありません。");
  });
});
