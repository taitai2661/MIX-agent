import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQueryClient } from "@tanstack/react-query";
import { Plus, RefreshCw, Sparkles } from "lucide-react";
import { useState } from "react";

import { Empty, ErrorBox, Field, Title, useRows } from "@/components/shared";

export function Models() {
  const rows = useRows("/models"),
    providers = useRows("/providers"),
    qc = useQueryClient();
  const [error, setError] = useState<unknown>(null),
    [open, setOpen] = useState(false);
  async function update(row: Row, cap: string, value: string) {
    try {
      await api("/models/" + row.id, "PATCH", {
        overrides: {
          ...row.data.overrides,
          [cap]: value === "unknown" ? null : value === "yes",
        },
      });
      qc.invalidateQueries({ queryKey: ["/models"] });
    } catch (e) {
      setError(e);
    }
  }
  async function verifyTools(row: Row) {
    try {
      await api("/models/" + row.id + "/verify-tools", "POST");
      qc.invalidateQueries({ queryKey: ["/models"] });
    } catch (e) {
      setError(e);
    }
  }
  async function updateContext(row: Row, raw: string) {
    try {
      await api("/models/" + row.id, "PATCH", {
        context_window_override: raw.trim() ? Number(raw) : null,
      });
      qc.invalidateQueries({ queryKey: ["/models"] });
      qc.invalidateQueries({ queryKey: ["/settings"] });
    } catch (e) {
      setError(e);
    }
  }
  return (
    <>
      <Title
        title="モデル"
        sub="対応機能を確認し、不明な項目は手動で設定できます。"
        action={
          <Button onClick={() => setOpen(!open)}>
            <Plus size={16} />
            手動追加
          </Button>
        }
      />
      <ErrorBox error={error || rows.error} />
      {open && (
        <form
          className="card form-grid"
          onSubmit={async (e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            try {
              await api("/models", "POST", {
                provider_id: f.get("provider_id"),
                model_id: f.get("model_id"),
                name: f.get("model_id"),
                context_window_override: f.get("context_window_override")
                  ? Number(f.get("context_window_override"))
                  : null,
              });
              qc.invalidateQueries({ queryKey: ["/models"] });
              setOpen(false);
            } catch (e) {
              setError(e);
            }
          }}
        >
          <Field label="Provider">
            <select name="provider_id" required>
              {providers.data?.map((p) => (
                <option value={p.id} key={p.id}>
                  {p.data.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="モデルID">
            <input name="model_id" required placeholder="ProviderのモデルID" />
          </Field>
          <Field label="Context Window" hint="不明なモデルをAutoで使う場合に設定します。">
            <input name="context_window_override" type="number" min="1024" max="10000000" step="1" placeholder="例: 128000" />
          </Field>
          <Button>追加</Button>
        </form>
      )}
      <div className="notice">
        モデル一覧の取得だけでは、Tool
        CallingやVision対応は保証されません。確認した機能を「対応」に設定してください。
      </div>
      {rows.data?.map((row) => (
        <div className="card model-card" key={row.id}>
          <div className="model-heading">
            <Sparkles size={20} />
            <div>
              <h3>{row.data.name || row.data.model_id}</h3>
              <small>
                {
                  providers.data?.find((p) => p.id === row.data.provider_id)
                    ?.data.name
                }{" "}
                ·{" "}
                {row.data.context_window
                  ? row.data.context_window.toLocaleString() + " context"
                  : "Context不明"}
                {row.data.context_source ? " · " + row.data.context_source : ""}
              </small>
            </div>
          </div>
          <div className="capabilities">
            {[
              ["tools", "Tool Calling"],
              ["vision", "Vision"],
              ["reasoning", "Reasoning"],
              ["structured_output", "Structured Output"],
            ].map(([key, label]) => {
              const value = Object.prototype.hasOwnProperty.call(
                row.data.overrides || {},
                key,
              )
                ? row.data.overrides[key]
                : row.data.capabilities?.[key];
              return (
                <Field label={label} key={key}>
                  <select
                    value={value == null ? "unknown" : value ? "yes" : "no"}
                    onChange={(e) => update(row, key, e.target.value)}
                  >
                    <option value="unknown">不明</option>
                    <option value="yes">対応</option>
                    <option value="no">非対応</option>
                  </select>
                </Field>
              );
            })}
          </div>
          <div className="model-context">
            <Field
              label="Context Window"
              hint={
                row.data.context_source === "manual"
                  ? "手動設定。空欄にして保存すると、API取得値に戻します。"
                  : row.data.context_source === "official_catalog"
                    ? "Provider公式カタログから補完されています。"
                    : row.data.context_source === "models_dev"
                      ? "models.devから補完されています。"
                      : row.data.context_source === "builtin"
                        ? "内蔵モデルDBから補完されています。"
                    : row.data.context_window
                      ? "Provider APIから取得しています。"
                      : "未設定のモデルは、安全のためAutoの候補から除外されます。"
              }
            >
              <input
                key={`${row.id}:${row.data.context_window_override ?? "api"}`}
                type="number"
                min="1024"
                max="10000000"
                step="1"
                defaultValue={row.data.context_window_override ?? ""}
                placeholder={row.data.context_window ? String(row.data.context_window) : "例: 128000"}
                onBlur={(e) => updateContext(row, e.currentTarget.value)}
              />
            </Field>
          </div>
          {!!row.data.metadata && (
            <details className="model-metadata">
              <summary>取得メタデータと根拠</summary>
              <small>信頼度: {row.data.context_confidence || "unknown"}</small>
              <pre>{JSON.stringify({ metadata: row.data.metadata, provider_metadata: row.data.provider_metadata }, null, 2)}</pre>
            </details>
          )}
          <div className="model-probe">
            <small>
              Tool Calling自動確認: {({
                supported: "対応",
                unsupported: "非対応",
                unknown: "未確認",
              } as Record<string, string>)[row.data.tool_probe?.status] || "未実施"}
              {row.data.tool_probe?.checked_at
                ? "（" + new Date(row.data.tool_probe.checked_at).toLocaleString() + "）"
                : ""}
            </small>
            <Button variant="outline" onClick={() => verifyTools(row)}>
              <RefreshCw size={14} />
              Tool Callingを再確認
            </Button>
          </div>
        </div>
      ))}
      {!rows.data?.length && (
        <Empty>Providerの「モデル取得」、または手動追加で登録できます。</Empty>
      )}
    </>
  );
}
