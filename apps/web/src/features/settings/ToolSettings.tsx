import { t } from "@/app/i18n";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title } from "@/components/shared";
import { useState } from "react";
import { Link } from "react-router-dom";

function useSettings() {
  return useQuery({ queryKey: ["/settings"], queryFn: () => api("/settings"), refetchInterval: (query) => query.state.data?.data.browser_install_status === "installing" ? 3000 : false });
}

export function BrowserSettings() {
  const query = useSettings();
  const qc = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  const [startingInstall, setStartingInstall] = useState(false);
  if (!query.data) return <ErrorBox error={query.error} />;
  const data = query.data.data;
  const status = startingInstall ? "installing" : String(data.browser_install_status || "not_installed");
  const progress = typeof data.browser_install_progress === "number" ? data.browser_install_progress : null;
  const stage = data.browser_install_stage;
  const install = async () => { setStartingInstall(true); setError(null); try { await api("/browser/enable", "POST"); await api("/browser/install", "POST"); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (e) { setError(e); } finally { setStartingInstall(false); } };
  const updateBrowser = async () => { setStartingInstall(true); setError(null); try { await api("/browser/update", "POST"); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (e) { setError(e); } finally { setStartingInstall(false); } };
  const payload = (form: FormData) => ({ browser_enabled: form.get("enabled") === "on", browser_timeout_ms: Number(form.get("timeout")), browser_locale: String(form.get("locale") || "ja-JP").trim(), browser_user_agent: String(form.get("userAgent") || "").trim(), browser_viewport_width: Number(form.get("viewportWidth")), browser_viewport_height: Number(form.get("viewportHeight")), browser_block_images: form.get("blockImages") === "on" });
  return <><Title title="Browser" sub={t("Docker内のPlaywright ChromiumでWebページを操作します。")} />
    <ErrorBox error={error || query.error} />
    <form className="card" onSubmit={async (event) => { event.preventDefault(); const form = new FormData(event.currentTarget); try { await api("/settings", "PUT", payload(form)); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (e) { setError(e); } }}>
      <Field label={t("Browserを有効にする")}><input name="enabled" type="checkbox" defaultChecked={data.browser_enabled !== false} /></Field>
      <section className="notice" role="status" aria-live="polite"><b>{t("Chromium導入状況:")}</b> {status === "installing" ? `${t("インストール中…")}${progress === null ? "" : ` ${progress}%`}` : t(({ not_installed: "未導入", ready: "利用可能", failed: "失敗" } as Record<string, string>)[status] || status)}<br />{status === "installing" ? stage || t("Chromiumを導入しています。") : data.browser_install_failure || t("Chromiumは専用ボリュームに保存されます。")}<br />{status === "ready" ? <><Button type="button" onClick={updateBrowser}>{t("Chromiumを更新")}</Button><span className="muted"> {t("現在のPlaywrightに対応する版を再導入します。新しい版にはDockerイメージの更新が必要です。")}</span></> : <Button type="button" onClick={install} disabled={status === "installing"}>{status === "installing" ? t("インストール中…") : status === "failed" ? t("Browserのみ再試行") : t("Browserを導入")}</Button>}</section>
      <section className="notice"><b>{t("実行方式:")}</b> {t("Playwright + Chromium（ヘッドレス）。専用Dockerコンテナからegress proxy経由で通信し、ホスト、Docker Socket、ローカルネットワークにはアクセスできません。")}</section>
      <Field label={t("操作タイムアウト")} hint={t("要素のクリック・入力・ページ読み込みを待つ上限です。3〜60秒。")}><input name="timeout" type="number" min="3000" max="60000" step="1000" defaultValue={data.browser_timeout_ms ?? 15000} /> ms</Field>
      <Field label={t("ページの言語")}><input name="locale" defaultValue={data.browser_locale || "ja-JP"} placeholder="ja-JP" /></Field>
      <Field label={t("画面サイズ")} hint={t("Webサイトへ渡す仮想ブラウザの表示領域です。")}><div className="inline-fields"><input name="viewportWidth" type="number" min="320" max="2560" defaultValue={data.browser_viewport_width ?? 1280} /><span>×</span><input name="viewportHeight" type="number" min="320" max="2560" defaultValue={data.browser_viewport_height ?? 720} /></div></Field>
      <Field label={t("User-Agent（任意）")} hint={t("空欄ならChromium標準のUser-Agentを使います。")}><input name="userAgent" defaultValue={data.browser_user_agent || ""} placeholder={t("標準を使用")} /></Field>
      <Field label={t("画像を読み込まない")} hint={t("帯域を抑え、テキスト中心の確認を速くします。")}><input name="blockImages" type="checkbox" defaultChecked={data.browser_block_images === true} /></Field>
      <p className="muted">{t("通信許可先は")}<Link to="/settings/network">{t("通信とネットワーク")}</Link>{t("で設定します。未設定なら公開ドメインへの通信を許可します。")}</p>
      <p className="muted">{t("ダウンロード、Service Worker、HTTP(S)以外のURLは無効です。Cookieやページ状態はRunnerの再起動・設定変更時に破棄され、バックアップにも含まれません。")}</p>
      <Button>{t("保存")}</Button>
    </form></>;
}

export function WebSearchSettings() {
  const query = useSettings(); const qc = useQueryClient(); const [error, setError] = useState<unknown>(null);
  if (!query.data) return <ErrorBox error={query.error} />;
  const data = query.data.data;
  type Backend = "ddgs" | "brave" | "tavily" | "exa" | "serper" | "searxng";
  const [backend, setBackend] = useState<Backend>(data.web_search_backend || "ddgs");
  const payload = (form: FormData) => ({ default_model_id: data.default_model_id || "", auto_model_ids: data.auto_model_ids || null, auto_retry_count: data.auto_retry_count ?? 3, setup_complete: data.setup_complete ?? false, browser_enabled: data.browser_enabled !== false, web_search_enabled: form.get("enabled") === "on", web_search_backend: String(form.get("backend") || "ddgs"), web_search_count: Number(form.get("count")), tool_settings: data.tool_settings || {}, allowed_domains: data.allowed_domains || [], searxng_url: backend === "searxng" ? String(form.get("searxng_url") || "").trim() : (data.searxng_url || ""), brave_api_key: backend === "brave" ? (String(form.get("key_brave") || "") || null) : null, tavily_api_key: backend === "tavily" ? (String(form.get("key_tavily") || "") || null) : null, exa_api_key: backend === "exa" ? (String(form.get("key_exa") || "") || null) : null, serper_api_key: backend === "serper" ? (String(form.get("key_serper") || "") || null) : null });
  return <><Title title={t("Web検索")} sub={t("Web検索の有効化、検索バックエンド、取得する結果数を設定します。")} /><ErrorBox error={error || query.error} />
    <form className="card" onSubmit={async (event) => { event.preventDefault(); const form = new FormData(event.currentTarget); try { await api("/settings", "PUT", payload(form)); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (e) { setError(e); } }}>
      <Field label={t("Web検索を有効にする")}><input name="enabled" type="checkbox" defaultChecked={data.web_search_enabled !== false} /></Field>
      <Field label={t("検索バックエンド")}><select name="backend" defaultValue={data.web_search_backend || "ddgs"} onChange={(event) => setBackend(event.target.value as Backend)}><option value="ddgs">{t("DDGS（APIキー不要）")}</option><option value="brave">Brave Search</option><option value="tavily">Tavily</option><option value="exa">Exa</option><option value="serper">Serper (Google)</option><option value="searxng">{t("SearXNG（自ホスト・キー不要）")}</option></select></Field>
      {backend === "brave" && <Field label="Brave Search API Key" hint={data.has_brave_secret_id ? t("保存済み。空欄なら現在のキーを保持します。") : t("Brave Search選択時はAPIキーが必要です。")}><input name="key_brave" type="password" /></Field>}
      {backend === "tavily" && <Field label="Tavily API Key" hint={data.has_tavily_secret_id ? t("保存済み。空欄なら現在のキーを保持します。") : t("Tavily選択時はAPIキーが必要です。")}><input name="key_tavily" type="password" /></Field>}
      {backend === "exa" && <Field label="Exa API Key" hint={data.has_exa_secret_id ? t("保存済み。空欄なら現在のキーを保持します。") : t("Exa選択時はAPIキーが必要です。")}><input name="key_exa" type="password" /></Field>}
      {backend === "serper" && <Field label="Serper API Key" hint={data.has_serper_secret_id ? t("保存済み。空欄なら現在のキーを保持します。") : t("Serper選択時はAPIキーが必要です。")}><input name="key_serper" type="password" /></Field>}
      {backend === "searxng" && <Field label="SearXNG URL" hint={t("公開HTTPSエンドポイントを指定します（例 https://searxng.example.com）。")}><input name="searxng_url" defaultValue={data.searxng_url || ""} placeholder="https://searxng.example.com" /></Field>}
      <Field label={t("検索結果件数")}><input name="count" type="number" min="1" max="20" defaultValue={data.web_search_count ?? 5} /></Field>
      <Button>{t("保存")}</Button>
    </form></>;
}
