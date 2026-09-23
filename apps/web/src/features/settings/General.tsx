import { api } from "@/app/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

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
    [saved, setSaved] = useState(false);
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
