import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { Button } from "@/components/button";
import { Empty, ErrorBox, Title } from "@/components/shared";
import type { components } from "@/generated/api";
import { useQuery } from "@tanstack/react-query";
import { Coins, Download, Server } from "lucide-react";
import { useState } from "react";

type UsageView = components["schemas"]["UsageView"];
type UsageGroup = UsageView["groups"][number];

const modeLabels: Record<string, string> = { chat: "Chat", thinking: "Thinking", agent: "Agent", tool: "Tool Calling" };
const sourceLabels: Record<string, string> = { manual: "手動設定", model_info: "model-info", unknown: "不明" };
const sortLabels: Record<string, string> = { tokens: "トークン順", cost: "コスト順", requests: "リクエスト順" };

function compact(value: number): string {
  if (value >= 1_000_000) return (value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1) + "M";
  if (value >= 1_000) return (value / 1_000).toFixed(value >= 10_000 ? 0 : 1) + "K";
  return String(value);
}

function money(value: number | null | undefined): string {
  if (value == null) return "—";
  if (value > 0 && value < 0.01) return "<$0.01";
  return "$" + value.toFixed(2);
}

function csvCell(value: unknown): string {
  const text = String(value ?? "");
  return /[",\r\n]/.test(text) ? '"' + text.replace(/"/g, '""') + '"' : text;
}

function downloadCsv(groups: UsageGroup[], days: number) {
  const header = ["model_id", "model_name", "provider", "mode", "requests", "input_tokens",
                  "output_tokens", "cached_tokens", "total_tokens", "cost_usd", "pricing_source"];
  const rows = [header, ...groups.map((g) => [
    g.model_id, g.model_name, g.provider_name || g.provider_id, g.mode, g.requests, g.input_tokens,
    g.output_tokens, g.cached_tokens, g.total_tokens, g.cost_usd ?? "", g.unpriced ? "unknown" : g.pricing_source,
  ])];
  // BOM keeps Excel reading UTF-8 Japanese model names correctly.
  const csv = "\uFEFF" + rows.map((row) => row.map(csvCell).join(",")).join("\r\n");
  const url = URL.createObjectURL(new Blob([csv], { type: "text/csv;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = `mix-usage-${days}d.csv`;
  link.click();
  URL.revokeObjectURL(url);
}

export function Usage() {
  const [days, setDays] = useState(30);
  const [sortBy, setSortBy] = useState<"tokens" | "cost" | "requests">("tokens");
  const query = useQuery<UsageView>({
    queryKey: ["/settings/usage", days],
    queryFn: () => api(`/settings/usage?days=${days}`),
  });
  const total = query.data?.total;
  const peak = Math.max(1, ...(query.data?.days || []).map((day) => day.total_tokens));
  const groups = [...(query.data?.groups || [])].sort((a, b) => {
    if (sortBy === "cost") return (b.cost_usd ?? -1) - (a.cost_usd ?? -1);
    if (sortBy === "requests") return b.requests - a.requests;
    return b.total_tokens - a.total_tokens;
  });
  const hasData = Boolean(total?.requests);
  return (
    <div className="page settings-statistics">
      <Title
        title={t("使用量")}
        sub={t("モデルごとのトークン使用量と、設定した単価に基づく概算コストを確認できます。保存されるのは集計に必要な最小限の記録だけです。")}
      />
      <div className="usage-toolbar">
        <div className="usage-range" role="group" aria-label={t("対象期間")}>
          {[7, 30, 90].map((value) => (
            <button key={value} type="button" className={value === days ? "active" : ""} onClick={() => setDays(value)}>
              {value}
              {t("日")}
            </button>
          ))}
        </div>
        <Button variant="outline" disabled={!hasData} onClick={() => downloadCsv(groups, days)}>
          <Download size={14} />
          {t("CSVをダウンロード")}
        </Button>
      </div>
      <ErrorBox error={query.error} />
      {query.isLoading ? (
        <div className="card"><p>{t("統計を読み込んでいます…")}</p></div>
      ) : !hasData ? (
        <Empty>{t("まだ使用履歴がありません。")}</Empty>
      ) : (
        total && (
          <>
            <section className="statistics-overview usage-overview">
              <div><span>{t("概算コスト")}</span><b>{money(total.cost_usd)}</b></div>
              <div><span>{t("入力Token")}</span><b>{compact(total.input_tokens)}</b></div>
              <div><span>{t("出力Token")}</span><b>{compact(total.output_tokens)}</b></div>
              <div><span>{t("合計Token")}</span><b>{compact(total.total_tokens)}</b></div>
              <div><span>{t("リクエスト")}</span><b>{total.requests}</b></div>
            </section>
            {(total.unpriced_requests > 0 || total.estimated_requests > 0) && (
              <p className="usage-note">
                {total.unpriced_requests > 0 && <span>{t("価格未設定でコスト集計外のリクエスト:")} {total.unpriced_requests}{t("件")}　</span>}
                {total.estimated_requests > 0 && <span>{t("Providerがusageを返さず推定した件数:")} {total.estimated_requests}{t("件")}</span>}
              </p>
            )}
            <section className="card statistics-section">
              <h2><Coins size={18} /> {t("日別のトークン使用量")}</h2>
              <div className="usage-days">
                {query.data?.days.map((day) => {
                  const cached = Math.min(day.input_tokens, Math.max(0, day.total_tokens - day.output_tokens));
                  const input = Math.max(0, day.input_tokens - cached);
                  return (
                    <div
                      className="usage-day"
                      key={day.date}
                      title={`${day.date} · ${day.total_tokens} tokens · ${t("入力")} ${day.input_tokens} / ${t("出力")} ${day.output_tokens}`}
                    >
                      <div className="usage-stack" style={{ height: `${Math.max(2, Math.round((day.total_tokens / peak) * 100))}%` }}>
                        <i className="out" style={{ flex: `${day.output_tokens} 1 0` }} />
                        <i className="in" style={{ flex: `${input} 1 0` }} />
                        <i className="cached" style={{ flex: `${cached} 1 0` }} />
                      </div>
                      <small>{day.date.slice(5)}</small>
                    </div>
                  );
                })}
              </div>
              <div className="usage-legend">
                <span><i className="out" />{t("出力")}</span>
                <span><i className="in" />{t("入力")}</span>
                <span><i className="cached" />{t("キャッシュ")}</span>
              </div>
            </section>
            {query.data?.providers && query.data.providers.length > 0 && (
              <section className="card statistics-section">
                <h2><Server size={18} /> {t("Provider別の使用量")}</h2>
                <div className="statistics-list">
                  {query.data.providers.map((item) => (
                    <article className="statistics-item" key={item.provider_id}>
                      <header>
                        <div><b>{item.provider_name || item.provider_id}</b></div>
                        <strong className={item.unpriced ? "usage-unpriced" : undefined}>
                          {item.unpriced ? t("価格未設定") : money(item.cost_usd)}
                        </strong>
                      </header>
                      <p>
                        {item.requests} {t("リクエスト")} · {t("入力")} {compact(item.input_tokens)} / {t("出力")} {compact(item.output_tokens)}
                        {item.cached_tokens > 0 && <> · {t("キャッシュ")} {compact(item.cached_tokens)}</>}
                      </p>
                      <div className="statistics-meta">
                        <span>{t("合計")} {compact(item.total_tokens)} tokens</span>
                      </div>
                    </article>
                  ))}
                </div>
              </section>
            )}
            <section className="card statistics-section">
              <h2>
                <Coins size={18} /> {t("モデル別の使用量")}
                <span className="usage-sort" role="group" aria-label={t("並び替え")}>
                  {(["tokens", "cost", "requests"] as const).map((value) => (
                    <button
                      key={value}
                      type="button"
                      className={value === sortBy ? "active" : ""}
                      onClick={() => setSortBy(value)}
                    >
                      {t(sortLabels[value])}
                    </button>
                  ))}
                </span>
              </h2>
              <div className="statistics-list">
                {groups.map((item) => (
                  <article className="statistics-item" key={item.key}>
                    <header>
                      <div><b>{item.model_name}</b><small>{modeLabels[item.mode] || item.mode} · {item.provider_name || item.provider_id}</small></div>
                      <strong className={item.unpriced ? "usage-unpriced" : undefined}>{item.unpriced ? t("価格未設定") : money(item.cost_usd)}</strong>
                    </header>
                    <p>
                      {item.requests} {t("リクエスト")} · {t("入力")} {compact(item.input_tokens)} / {t("出力")} {compact(item.output_tokens)}
                      {item.cached_tokens > 0 && <> · {t("キャッシュ")} {compact(item.cached_tokens)}</>}
                    </p>
                    <div className="statistics-meta">
                      <span>{t("単価:")} {t(sourceLabels[item.pricing_source] || item.pricing_source)}</span>
                      <span>{t("合計")} {compact(item.total_tokens)} tokens</span>
                    </div>
                  </article>
                ))}
              </div>
            </section>
          </>
        )
      )}
    </div>
  );
}
