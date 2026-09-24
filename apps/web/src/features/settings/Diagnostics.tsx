import { getLanguage, t } from "@/app/i18n";
import { api } from "@/app/api";
import { ErrorBox, Title } from "@/components/shared";
import type { components } from "@/generated/api";
import { useQuery } from "@tanstack/react-query";
import { Activity, AlertTriangle, CheckCircle2, CircleHelp, RefreshCw } from "lucide-react";
import { Link } from "react-router-dom";

type DiagnosticsView = components["schemas"]["DiagnosticsView"];

const serviceLabels: Record<string, string> = {
  database: "データベース",
  execution: "実行Runner",
  mcp: "MCP Runner",
  browser: "Browser Runner",
  mcp_manager: "MCP管理サービス",
  browser_install: "Browser導入",
};
const stateLabels: Record<string, string> = {
  ok: "正常",
  configured: "設定済み（接続未確認）",
  not_configured: "未設定",
  ready: "利用可能",
  installing: "導入中",
  failed: "失敗",
  not_installed: "未導入",
  unknown: "状態不明",
};

function serviceTone(status: string) {
  if (status === "ok" || status === "ready") return "ok";
  if (status === "not_installed" || status === "unknown" || status === "installing") return "pending";
  return "warning";
}

export function Diagnostics() {
  const query = useQuery<DiagnosticsView>({
    queryKey: ["/settings/diagnostics"],
    queryFn: () => api("/settings/diagnostics"),
    refetchInterval: 60_000,
  });
  const data = query.data;
  const hasWarnings = Boolean(data && (
    data.runs.stale_active || data.runs.failed_30d || data.runs.interrupted_30d ||
    Object.values(data.services).some((state) => ["not_configured", "failed"].includes(state))
  ));

  return <div className="page diagnostics-page">
    <Title title={t("障害診断")} sub={t("サービスの設定状態と最近の実行失敗を確認します。会話内容やエラー本文は表示しません。")}
      action={<button className="button btn-outline" type="button" disabled={query.isFetching} onClick={() => void query.refetch()}><RefreshCw size={15} /> {t("今すぐ確認")}</button>} />
    <ErrorBox error={query.error} />
    {!data ? <section className="card"><p>{query.isLoading ? t("診断情報を確認しています…") : t("診断情報を取得できませんでした。")}</p></section> : <>
      <section className={`diagnostics-banner ${hasWarnings ? "warning" : "ok"}`}>
        {hasWarnings ? <AlertTriangle size={20} /> : <CheckCircle2 size={20} />}
        <div><b>{hasWarnings ? t("確認が必要な項目があります") : t("設定上の問題は見つかりませんでした")}</b>
          <small>{t("最終確認:")} {new Date(data.checked_at).toLocaleString(getLanguage() === "en" ? "en-US" : "ja-JP")}</small></div>
      </section>

      <section className="card diagnostics-section">
        <h2><Activity size={18} /> {t("サービス状態")}</h2>
        <div className="diagnostics-services">{Object.entries(data.services).map(([key, status]) => <article className="diagnostics-service" key={key}>
          <span className={`diagnostics-dot ${serviceTone(status)}`} />
          <div><b>{t(serviceLabels[key] || key)}</b><small>{t(stateLabels[status] || status)}</small></div>
          {status === "not_configured" && <Link to={key === "browser" ? "/settings/browser" : key === "mcp" || key === "mcp_manager" ? "/settings/mcp" : "/settings/tools"}>{t("設定を確認")}</Link>}
        </article>)}</div>
        <p className="diagnostics-note"><CircleHelp size={14} /> {t("Runnerは認証情報の設定有無を表示します。実際のネットワーク疎通は確認していません。")}</p>
      </section>

      <section className="diagnostics-run-summary">
        <article><span>{t("進行中")}</span><b>{data.runs.active}</b></article>
        <article><span>{t("失敗（30日）")}</span><b>{data.runs.failed_30d}</b></article>
        <article><span>{t("中断（30日）")}</span><b>{data.runs.interrupted_30d}</b></article>
        <article className={data.runs.stale_active ? "has-warning" : ""}><span>{t("30分以上停滞")}</span><b>{data.runs.stale_active}</b></article>
      </section>

      <section className="card diagnostics-section">
        <h2><AlertTriangle size={18} /> {t("最近の失敗・中断")}</h2>
        {data.runs.recent_failures.length ? <div className="diagnostics-failures">{data.runs.recent_failures.map((run, index) => <article key={`${run.created_at}-${index}`}>
          <b>{run.status === "failed" ? t("失敗") : t("中断")} · {run.mode}</b><time dateTime={run.created_at}>{new Date(run.created_at).toLocaleString(getLanguage() === "en" ? "en-US" : "ja-JP")}</time>
        </article>)}</div> : <p className="diagnostics-empty">{t("直近の失敗・中断はありません。")}</p>}
      </section>
    </>}
  </div>;
}
