import { api } from "@/app/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";

import { ErrorBox, Field, Title, useRows } from "@/components/shared";
import { ThemeController } from "@/components/theme";

export function General() {
  const setting = useQuery({
      queryKey: ["/settings"],
      queryFn: () => api("/settings"),
    }),
    models = useRows("/models"),
    qc = useQueryClient();
  const [error, setError] = useState<unknown>(null),
    [saved, setSaved] = useState(false),
    [modelQuery, setModelQuery] = useState(""),
    [showSelectedOnly, setShowSelectedOnly] = useState(false),
    [selectedModelIds, setSelectedModelIds] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (setting.data) setSelectedModelIds(new Set(setting.data.data.auto_model_ids || []));
  }, [setting.data, setting.dataUpdatedAt]);

  const filteredModels = useMemo(() => {
    const query = modelQuery.trim().toLocaleLowerCase();
    return (models.data || []).filter((model) => {
      const name = model.data.name || model.data.model_id || "";
      return (!showSelectedOnly || selectedModelIds.has(model.id)) &&
        (!query || name.toLocaleLowerCase().includes(query));
    });
  }, [modelQuery, models.data, selectedModelIds, showSelectedOnly]);

  const updateVisibleSelections = (checked: boolean) => {
    setSelectedModelIds((current) => {
      const next = new Set(current);
      filteredModels.forEach((model) => checked ? next.add(model.id) : next.delete(model.id));
      return next;
    });
  };

  const toggleModel = (id: string) => {
    setSelectedModelIds((current) => {
      const next = new Set(current);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  };
  return (
    <>
      <Title
        title="一般・通信"
        sub="既定モデルと、外部サービスへの通信を設定します。"
      />
      <ErrorBox error={error || setting.error} />
      <section className="card theme-card">
        <div>
          <h3>表示テーマ</h3>
          <p>ライト、ダーク、またはOSの設定に合わせて表示を切り替えます。</p>
        </div>
        <ThemeController />
      </section>
      {setting.data && (
        <form
          className="card"
          key={setting.dataUpdatedAt}
          onSubmit={async (e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            try {
              await api("/settings", "PUT", {
                default_model_id: f.get("default_model_id"),
                auto_model_ids: [...selectedModelIds],
                auto_retry_count: Number(f.get("auto_retry_count")),
                setup_complete: setting.data.data.setup_complete,
                allowed_domains: String(f.get("domains"))
                  .split("\n")
                  .map((x) => x.trim())
                  .filter(Boolean),
                brave_api_key: f.get("brave_api_key") || null,
                web_search_backend: setting.data.data.web_search_backend || "ddgs",
              });
              setSaved(true);
              qc.invalidateQueries({ queryKey: ["/settings"] });
            } catch (e) {
              setError(e);
            }
          }}
        >
          <Field label="既定モデル">
            <select
              name="default_model_id"
              defaultValue={setting.data.data.default_model_id || ""}
            >
              <option value="">未設定</option>
              <option value="auto">Auto</option>
              {models.data?.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.data.name || m.data.model_id}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Autoで使用可能なモデル" hint="Autoは、この中から必要なCapabilityと過去の評価をもとに選択します。">
            <div className="auto-model-picker">
              <div className="auto-model-toolbar">
                <input aria-label="Auto候補を検索" className="auto-model-search" value={modelQuery} onChange={(event) => setModelQuery(event.target.value)} placeholder="モデルを検索" type="search" />
                <span className="auto-model-count">{selectedModelIds.size}件を選択中</span>
              </div>
              <div className="auto-model-actions">
                <button type="button" className="btn-ghost" onClick={() => setShowSelectedOnly((value) => !value)} aria-pressed={showSelectedOnly}>{showSelectedOnly ? "すべて表示" : "選択済みのみ"}</button>
                <button type="button" className="btn-ghost" onClick={() => updateVisibleSelections(true)} disabled={!filteredModels.length}>すべて選択</button>
                <button type="button" className="btn-ghost" onClick={() => updateVisibleSelections(false)} disabled={!filteredModels.length}>すべて解除</button>
              </div>
              <div className="auto-model-list" aria-label="Autoで使用可能なモデル">
                {filteredModels.map((model) => (
                  <label className="auto-model-option" key={model.id}>
                    <input checked={selectedModelIds.has(model.id)} onChange={() => toggleModel(model.id)} type="checkbox" />
                    <span className="auto-model-name">{model.data.name || model.data.model_id}</span>
                    <small>{model.data.context_window ? `${model.data.context_window.toLocaleString()} context` : "Context Window未設定"}</small>
                  </label>
                ))}
                {!filteredModels.length && <p className="auto-model-empty">該当するモデルがありません。</p>}
              </div>
            </div>
          </Field>
          <Field label="Auto実行の再試行回数" hint="一時的なProvider障害時に、未使用のAuto候補へ切り替える回数です。0なら再試行しません。モデル未検出・認証・設定エラーは再試行しません。">
            <input name="auto_retry_count" type="number" min="0" step="1" defaultValue={setting.data.data.auto_retry_count ?? 3} />
          </Field>
          <Field
            label="Brave Search API Key"
            hint={
              setting.data.data.has_brave_secret_id
                ? "保存済み。空欄なら保持します。Tavily/Exa/SerperのキーはWeb検索設定から登録します。"
                : "Brave利用時に必要です。他バックエンドのキーはWeb検索設定から登録します。"
            }
          >
            <input name="brave_api_key" type="password" />
          </Field>
          <Field
            label="Runner通信許可先（1行1ドメイン）"
            hint="完全一致。サブドメインは別途指定します。未設定なら公開ドメインへの通信を許可します。入力すると指定ドメインだけに制限します。"
          >
            <textarea
              name="domains"
              rows={6}
              defaultValue={setting.data.data.allowed_domains?.join("\n")}
              placeholder={"example.com\nregistry.npmjs.org"}
            />
          </Field>
          <div className="notice">
            API
            Providerへの接続設定とは別です。Terminal・Browser・MCPの通信に適用します。LAN、localhost、metadata
            endpointは許可できません。
          </div>
          <Button>保存</Button>
          {saved && <span className="success">保存しました</span>}
        </form>
      )}
    </>
  );
}
