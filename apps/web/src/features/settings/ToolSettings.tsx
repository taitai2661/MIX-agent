import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title } from "@/components/shared";
import { useState } from "react";

function useSettings() {
  return useQuery({ queryKey: ["/settings"], queryFn: () => api("/settings") });
}

export function BrowserSettings() {
  const query = useSettings();
  const qc = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  if (!query.data) return <ErrorBox error={query.error} />;
  const data = query.data.data;
  const payload = (form: FormData) => ({ default_model_id: data.default_model_id || "", auto_model_ids: data.auto_model_ids || null, auto_retry_count: data.auto_retry_count ?? 3, setup_complete: data.setup_complete ?? false, browser_enabled: form.get("enabled") === "on", web_search_enabled: data.web_search_enabled !== false, web_search_count: data.web_search_count ?? 5, tool_settings: data.tool_settings || {}, allowed_domains: String(form.get("domains")).split("\n").map((x) => x.trim()).filter(Boolean), brave_api_key: null });
  return <><Title title="Browser" sub="Browser Toolの利用可否と、アクセスを許可するドメインを設定します。" />
    <ErrorBox error={error || query.error} />
    <form className="card" onSubmit={async (event) => { event.preventDefault(); const form = new FormData(event.currentTarget); try { await api("/settings", "PUT", payload(form)); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (e) { setError(e); } }}>
      <Field label="Browserを有効にする"><input name="enabled" type="checkbox" defaultChecked={data.browser_enabled !== false} /></Field>
      <Field label="許可ドメイン（1行1ドメイン）" hint="BrowserとRunnerの外向き通信に適用します。"><textarea name="domains" rows={8} defaultValue={data.allowed_domains?.join("\n")} placeholder="example.com" /></Field>
      <Button>保存</Button>
    </form></>;
}

export function WebSearchSettings() {
  const query = useSettings(); const qc = useQueryClient(); const [error, setError] = useState<unknown>(null);
  if (!query.data) return <ErrorBox error={query.error} />;
  const data = query.data.data;
  const payload = (form: FormData) => ({ default_model_id: data.default_model_id || "", auto_model_ids: data.auto_model_ids || null, auto_retry_count: data.auto_retry_count ?? 3, setup_complete: data.setup_complete ?? false, browser_enabled: data.browser_enabled !== false, web_search_enabled: form.get("enabled") === "on", web_search_count: Number(form.get("count")), tool_settings: data.tool_settings || {}, allowed_domains: data.allowed_domains || [], brave_api_key: String(form.get("key") || "") || null });
  return <><Title title="Web検索" sub="Web検索の有効化、Brave Search、取得する結果数を設定します。" /><ErrorBox error={error || query.error} />
    <form className="card" onSubmit={async (event) => { event.preventDefault(); const form = new FormData(event.currentTarget); try { await api("/settings", "PUT", payload(form)); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (e) { setError(e); } }}>
      <Field label="Web検索を有効にする"><input name="enabled" type="checkbox" defaultChecked={data.web_search_enabled !== false} /></Field>
      <Field label="Brave Search API Key" hint={data.has_brave_secret_id ? "保存済み。空欄なら現在のキーを保持します。" : "未設定の場合、Web検索は実行できません。"}><input name="key" type="password" /></Field>
      <Field label="検索結果件数"><input name="count" type="number" min="1" max="20" defaultValue={data.web_search_count ?? 5} /></Field>
      <Button>保存</Button>
    </form></>;
}
