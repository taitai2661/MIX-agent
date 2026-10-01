import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQueryClient } from "@tanstack/react-query";
import { Plus, RefreshCw, Search, Sparkles } from "lucide-react";
import { useMemo, useState } from "react";

import { Empty, ErrorBox, Field, Title, useRows } from "@/components/shared";

export function Models() {
  const rows = useRows("/models"),
    providers = useRows("/providers"),
    qc = useQueryClient();
  const [error, setError] = useState<unknown>(null),
    [open, setOpen] = useState(false),
    [query, setQuery] = useState("");
  const filteredRows = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return rows.data || [];
    return (rows.data || []).filter((row) => {
      const providerName = providers.data?.find((provider) => provider.id === row.data.provider_id)?.data.name || "";
      return [row.data.name, row.data.model_id, providerName]
        .some((value) => String(value || "").toLocaleLowerCase().includes(needle));
    });
  }, [providers.data, query, rows.data]);
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
  async function updatePricing(row: Row, field: string, raw: string) {
    const previous = row.data.pricing_override || {};
    const next: Record<string, number | null> = {
      input: previous.input ?? null,
      output: previous.output ?? null,
      cache_read: previous.cache_read ?? null,
    };
    next[field] = raw.trim() ? Number(raw) : null;
    try {
      await api("/models/" + row.id, "PATCH", {
        pricing_override: Object.values(next).some((value) => value != null) ? next : null,
      });
      qc.invalidateQueries({ queryKey: ["/models"] });
      qc.invalidateQueries({ queryKey: ["/settings/usage"] });
    } catch (e) {
      setError(e);
    }
  }
  return (
    <>
      <Title
        title={t("モデル")}
        sub={t("対応機能を確認し、不明な項目は手動で設定できます。")}
        action={
          <Button onClick={() => setOpen(!open)}>
            <Plus size={16} />
            {t("手動追加")}
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
          <Field label={t("モデルID")}>
            <input name="model_id" required placeholder={t("ProviderのモデルID")} />
          </Field>
          <Field label="Context Window" hint={t("不明なモデルをAutoで使う場合に設定します。")}>
            <input name="context_window_override" type="number" min="1024" max="10000000" step="1" placeholder={t("例: 128000")} />
          </Field>
          <Button>{t("追加")}</Button>
        </form>
      )}
      <div className="notice">
        {t("モデル一覧の取得だけでは、Tool CallingやVision対応は保証されません。確認した機能を「対応」に設定してください。")}
      </div>
      <label className="tool-search models-search">
        <Search size={15} />
        <input
          aria-label={t("モデルを検索")}
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("モデルを検索")}
        />
      </label>
      {filteredRows.map((row) => (
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
                  : t("Context不明")}
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
                    <option value="unknown">{t("不明")}</option>
                    <option value="yes">{t("対応")}</option>
                    <option value="no">{t("非対応")}</option>
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
                  ? t("手動設定。空欄にして保存すると、API取得値に戻します。")
                  : row.data.context_source === "model_info"
                    ? t("model-infoから補完されています。")
                    : row.data.context_window
                      ? t("Provider APIから取得しています。")
                      : t("未設定のモデルは、安全のためAutoの候補から除外されます。")
              }
            >
              <input
                key={`${row.id}:${row.data.context_window_override ?? "api"}`}
                type="number"
                min="1024"
                max="10000000"
                step="1"
                defaultValue={row.data.context_window_override ?? ""}
                placeholder={row.data.context_window ? String(row.data.context_window) : t("例: 128000")}
                onBlur={(e) => updateContext(row, e.currentTarget.value)}
              />
            </Field>
          </div>
          {!!row.data.metadata && (
            <details className="model-metadata">
              <summary>{t("取得メタデータと根拠")}</summary>
              <small>{t("信頼度:")} {row.data.context_confidence || "unknown"}</small>
              <pre>{JSON.stringify({ metadata: row.data.metadata, provider_metadata: row.data.provider_metadata }, null, 2)}</pre>
            </details>
          )}
          <div className="model-probe">
            <small>
              {t("自動確認:")}{" "}
              {([
                ["tools", "Tool Calling"],
                ["vision", "Vision"],
                ["reasoning", "Reasoning"],
              ] as const)
                .map(([key, label]) => {
                  const status = key === "tools" ? row.data.tool_probe?.status : row.data.tool_probe?.[key];
                  const text =
                    status === "supported"
                      ? t("対応")
                      : status === "unsupported"
                        ? t("非対応")
                        : status === "unknown"
                          ? t("未確認")
                          : t("未実施");
                  return `${label}: ${text}`;
                })
                .join("・")}
              {row.data.tool_probe?.checked_at
                ? "（" + new Date(row.data.tool_probe.checked_at).toLocaleString() + "）"
                : ""}
            </small>
            <Button variant="outline" onClick={() => verifyTools(row)}>
              <RefreshCw size={14} />
              {t("対応機能を再確認")}
            </Button>
          </div>
        </div>
      ))}
      {!rows.data?.length && (
        <Empty>{t("Providerの「モデル取得」、または手動追加で登録できます。")}</Empty>
      )}
      {!!rows.data?.length && !filteredRows.length && (
        <Empty>{t("該当するモデルがありません。")}</Empty>
      )}
    </>
  );
}
