import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { Empty, ErrorBox, Title } from "@/components/shared";
import type { components } from "@/generated/api";
import { useQuery } from "@tanstack/react-query";
import { Coins } from "lucide-react";
import { useState } from "react";

type UsageView = components["schemas"]["UsageView"];

const modeLabels: Record<string, string> = { chat: "Chat", thinking: "Thinking", agent: "Agent", tool: "Tool Calling" };
const sourceLabels: Record<string, string> = { manual: "手動設定", models_dev: "models.dev", unknown: "不明" };

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

export function Usage() {
  const [days, setDays] = useState(30);
  const query = useQuery<UsageView>({
    queryKey: ["/settings/usage", days],
    queryFn: () => api(`/settings/usage?days=${days}`),
  });
  const total = query.data?.total;
  const peak = Math.max(1, ...(query.data?.days || []).map((day) => day.total_tokens));
  return (
    <div className="page settings-statistics">
      <Title
        title={t("使用量")}
        sub={t("モデルごとのトークン使用量と、設定した単価に基づく概算コストを確認できます。保存されるのは集計に必要な最小限の記録だけです。")}
      />
      <div className="usage-range" role="group" aria-label={t("対象期間")}>
        {[7, 30, 90].map((value) => (
          <button key={value} type="button" className={value === days ? "active" : ""} onClick={() => setDays(value)}>
            {value}
            {t("日")}
          </button>
        ))}
      </div>
      <ErrorBox error={query.error} />
      {query.isLoading ? (
        <div className="card"><p>{t("統計を読み込んでいます…")}</p></div>
      ) : !total?.requests ? (
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
                {query.data?.days.map((day) => (
                  <div className="usage-day" key={day.date} title={`${day.date} · ${day.total_tokens} tokens`}>
                    <i style={{ height: `${Math.max(2, Math.round((day.total_tokens / peak) * 100))}%` }} />
                    <small>{day.date.slice(5)}</small>
                  </div>
                ))}
              </div>
            </section>
            <section className="card statistics-section">
              <h2><Coins size={18} /> {t("モデル別の使用量")}</h2>
              <div className="statistics-list">
                {query.data?.groups.map((item) => (
                  <article className="statistics-item" key={item.key}>
                    <header>
                      <div><b>{item.model_name}</b><small>{modeLabels[item.mode] || item.mode} · {item.provider_id}</small></div>
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
