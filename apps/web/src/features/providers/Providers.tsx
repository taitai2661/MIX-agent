import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Plus, RefreshCw, Server } from "lucide-react";
import { useState, type FormEvent } from "react";

import { Empty, ErrorBox, Field, Title, useRows } from "@/components/shared";

type ProviderPreset = {
  id: string;
  name: string;
  category: string;
  kind: string;
  default_url: string;
  api_key_required: boolean;
  allow_private_default: boolean;
  extra_config_schema: { key: string; label: string; required?: boolean }[];
};

export function Providers() {
  const rows = useRows("/providers"),
    qc = useQueryClient();
  const presets = useQuery<ProviderPreset[]>({
    queryKey: ["/provider-presets"],
    queryFn: () => api("/provider-presets"),
  });
  const [editing, setEditing] = useState<Row | null>(null),
    [open, setOpen] = useState(false),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false),
    [filter, setFilter] = useState(""),
    [selectedPreset, setSelectedPreset] = useState<ProviderPreset | null>(null);
  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    setBusy(true);
    setError(null);
    try {
      const result = await api<{ model_sync?: { status: string; count: number; auto_count: number; error?: string } }>(
        "/providers" + (editing ? "/" + editing.id : ""),
        editing ? "PATCH" : "POST",
        {
          name: f.get("name"),
          kind: f.get("kind") || null,
          preset_id: f.get("preset_id") || null,
          base_url: f.get("base_url"),
          api_key: f.get("api_key") || null,
          allow_private: f.has("allow_private"),
          rate_limit_rpm: Number(f.get("rate_limit_rpm") || 0),
          rate_limit_period: f.get("rate_limit_period") || "minute",
          extra_config: Object.fromEntries((selectedPreset?.extra_config_schema || []).map((field) => [
            field.key, f.get("extra_config." + field.key) || "",
          ])),
        },
      );
      setOpen(false);
      const sync = result.model_sync;
      if (sync?.status === "ok") {
        setNotice(`${sync.count}件のモデルを取得し、${sync.auto_count}件をAuto候補へ追加しました`);
      } else if (sync?.status === "failed") {
        const missing = sync.error?.startsWith("missing_extra_config:")
          ? "追加設定が不足しています: " + sync.error.slice("missing_extra_config:".length)
          : "モデル取得に失敗しました。";
        setNotice("Providerは保存しましたが、" + missing + " 接続を確認して「モデル取得」で再試行してください。");
      }
      qc.invalidateQueries({ queryKey: ["/providers"] });
      qc.invalidateQueries({ queryKey: ["/models"] });
      qc.invalidateQueries({ queryKey: ["/settings"] });
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  function openForm(row: Row | null) {
    setEditing(row);
    setFilter("");
    setSelectedPreset(row ? presets.data?.find((preset) => preset.id === row.data.preset_id) || null : null);
    setOpen(true);
  }
  const filtered = presets.data?.filter((preset) =>
    (preset.name + preset.category).toLocaleLowerCase().includes(filter.toLocaleLowerCase()),
  );
  async function action(row: Row, name: string) {
    setBusy(true);
    setError(null);
    try {
      const r = await api("/providers/" + row.id + "/" + name, "POST");
      setNotice(
        name === "test"
          ? "接続できました（推論テストは行っていません）"
          : r.count + "件のモデルを取得し、" + (r.auto_count || 0) + "件をAuto候補へ追加しました",
      );
      qc.invalidateQueries({ queryKey: ["/models"] });
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <Title
        title="AI Providers"
        sub="Providerを保存すると、モデル一覧とAuto候補を自動設定します。"
        action={
          <Button
            onClick={() => {
              openForm(null);
            }}
          >
            <Plus size={16} />
            Providerを追加
          </Button>
        }
      />
      <div className="notice">
        <KeyRound size={16} />
        API Keyは暗号化して保存され、保存後は再表示されません。
      </div>
      <ErrorBox error={error || rows.error} />
      {notice && <p className="success">{notice}</p>}
      {open && (
        <form
          className="card form-grid"
          key={editing?.id || "new"}
          onSubmit={submit}
        >
          <Field label="Providerを選ぶ" hint="プリセットは接続方式を安全に固定します。">
            <input
              type="search"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Providerを検索"
              aria-label="Providerを検索"
            />
            <select
              name="preset_id"
              defaultValue={editing?.data.preset_id || ""}
              required={!editing?.data.kind}
              onChange={(e) => {
                const preset = presets.data?.find((x) => x.id === e.target.value);
                const form = e.currentTarget.form;
                if (!preset || !form) return;
                setSelectedPreset(preset);
                const named = form.elements.namedItem("name") as HTMLInputElement | null;
                const url = form.elements.namedItem("base_url") as HTMLInputElement | null;
                const kind = form.elements.namedItem("kind") as HTMLSelectElement | null;
                const privateInput = form.elements.namedItem("allow_private") as HTMLInputElement | null;
                if (named && !editing) named.value = preset.id === "custom" ? "" : preset.name;
                if (url && preset.default_url) url.value = preset.default_url;
                if (kind) kind.value = preset.kind;
                if (privateInput) privateInput.checked = preset.allow_private_default;
              }}
            >
              <option value="">従来の接続設定（編集時のみ）</option>
              {Object.entries(
                (filtered || []).reduce<Record<string, ProviderPreset[]>>((groups, item) => {
                  (groups[item.category] ||= []).push(item);
                  return groups;
                }, {}),
              ).map(([category, items]) => (
                <optgroup key={category} label={category}>
                  {items.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
                </optgroup>
              ))}
            </select>
          </Field>
          <Field label="表示名">
            <input
              name="name"
              required
              defaultValue={editing?.data.name}
              placeholder="My OpenAI"
            />
          </Field>
          <Field label="カスタム接続方式" hint="カスタム以外では選択したプリセットの方式が使用されます。">
            <select name="kind" defaultValue={editing?.data.kind || "compatible"}>
              <option value="compatible">OpenAI互換</option>
              <option value="anthropic">Anthropic Messages</option>
              <option value="gemini">Gemini GenerateContent</option>
              <option value="openai">OpenAI Responses（従来設定）</option>
              <option value="openrouter">OpenRouter（従来設定）</option>
              <option value="ollama">Ollama（従来設定）</option>
              <option value="lmstudio">LM Studio（従来設定）</option>
            </select>
          </Field>
          <Field label="Base URL" hint="カスタムとURL未設定のプリセットでは必須です。">
            <input
              name="base_url"
              defaultValue={editing?.data.base_url}
              placeholder="https://api.example.com/v1"
            />
          </Field>
          <Field
            label="API Key"
            hint={
              editing
                ? "空欄なら現在のKeyを保持します。"
                : "ローカルモデルでは不要な場合があります。"
            }
          >
            <input name="api_key" type="password" autoComplete="off" />
          </Field>
          {selectedPreset?.extra_config_schema.map((field) => (
            <Field key={field.key} label={field.label} hint="このProvider固有の接続設定です。">
              <input name={"extra_config." + field.key} required={field.required}
                defaultValue={editing?.data.extra_config?.[field.key] || ""} />
            </Field>
          ))}
          <details className="provider-advanced">
            <summary>詳細設定</summary>
            <Field label="要求回数の上限" hint="Providerへの要求を制限します。0は無制限です。">
              <div className="rate-limit-input">
                <input name="rate_limit_rpm" type="number" min="0" max="10000" step="1"
                  defaultValue={editing?.data.rate_limit_rpm || 0} />
                <select name="rate_limit_period" defaultValue={editing?.data.rate_limit_period || "minute"}>
                  <option value="minute">回 / 分</option>
                  <option value="second">回 / 秒</option>
                </select>
              </div>
            </Field>
          </details>
          <label className="check">
            <input
              type="checkbox"
              name="allow_private"
              defaultChecked={editing?.data.allow_private}
            />
            明示したLAN・ローカル接続先を許可
          </label>
          <div className="form-actions">
            <Button disabled={busy}>保存</Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setOpen(false)}
            >
              キャンセル
            </Button>
          </div>
        </form>
      )}
      {rows.data?.map((row) => (
        <div className="card provider-card" key={row.id}>
          <div className="provider-icon">
            <Server size={22} />
          </div>
          <div className="grow">
            <h3>{row.data.name}</h3>
            <p>
              {row.data.kind} <span>·</span> {row.data.base_url}
            </p>
            <small>
              {row.data.has_secret_id ? "API Key 保存済み" : "API Key なし"}
            </small>
          </div>
          <div className="row-actions">
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => action(row, "test")}
            >
              接続テスト
            </Button>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => action(row, "sync-models")}
            >
              <RefreshCw size={14} />
              モデル取得
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                openForm(row);
              }}
            >
              編集
            </Button>
          </div>
        </div>
      ))}
      {!rows.data?.length && !open && (
        <Empty>Providerを追加して、チャットを始めましょう。</Empty>
      )}
    </>
  );
}
