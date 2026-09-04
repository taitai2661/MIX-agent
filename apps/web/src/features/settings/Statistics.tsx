import { api } from "@/app/api";
import { Empty, ErrorBox, Title } from "@/components/shared";
import type { components } from "@/generated/api";
import { useQuery } from "@tanstack/react-query";
import { BarChart3 } from "lucide-react";

const scopeLabels: Record<string, string> = { chat: "Chat", thinking: "Thinking", tool: "Tool Calling" };
const failureLabels: Record<string, string> = { timeout: "タイムアウト", rate_limit: "レート制限", provider_5xx: "Provider障害", context: "Context不足", tool: "Tool対応", auth: "認証", not_found: "未検出", other: "その他" };
type StatisticsView = components["schemas"]["StatisticsView"];

export function Statistics() {
  const query = useQuery<StatisticsView>({ queryKey: ["/settings/statistics"], queryFn: () => api("/settings/statistics") });
  const total = query.data?.total;
  return <div className="page settings-statistics"><Title title="統計" sub="直近30日間のモデル利用状況・失敗傾向・回答生成速度を確認できます。保存されるのは集計に必要な最小限の実行結果だけです。" /><ErrorBox error={query.error} />
    {query.isLoading ? <div className="card"><p>統計を読み込んでいます…</p></div> : !total?.total && !total?.tps_count ? <Empty>まだ実行履歴がありません。</Empty> : total && <><section className="statistics-overview"><div><span>実行回数</span><b>{total.total}</b></div><div><span>成功率</span><b>{(100 - total.failure_rate).toFixed(1)}%</b></div><div><span>平均TPS</span><b>{total.tokens_per_second != null ? `${total.tokens_per_second}` : "—"}</b></div><div><span>TPS算出</span><b>{total.tps_count} 回</b></div><div><span>対象期間</span><b>30日</b></div></section><section className="card statistics-section"><h2><BarChart3 size={18} /> モデル別の詳細</h2><div className="statistics-list">{query.data?.groups.map((item) => <article className="statistics-item" key={item.key}><header><div><b>{item.model_name}</b><small>{scopeLabels[item.scope] || item.scope} · {item.provider_id}</small></div>{item.total > 0 && <strong>{item.failure_rate.toFixed(1)}% 失敗</strong>}</header><div className="statistics-bar"><i style={{ width: `${100 - item.failure_rate}%` }} /></div>{item.total > 0 && <p>{item.total} 回中 {item.success} 回成功 / {item.failure} 回失敗</p>}<div className="statistics-meta">{Object.entries(item.classifications || {}).map(([key, value]) => <span key={key}>{failureLabels[key] || key}: {value}</span>)}{item.tokens_per_second != null && <span>平均: {item.tokens_per_second} tokens/s ({item.tps_count} 回)</span>}{item.first_output_ms != null && <span>初回出力: {item.first_output_ms}ms</span>}{item.completion_ms != null && <span>完了: {item.completion_ms}ms</span>}</div></article>)}</div></section></>}
  </div>;
}
