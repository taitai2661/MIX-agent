import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ConfirmButton } from "@/components/modal";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Plus, RefreshCw, Server } from "lucide-react";
import { useCallback, useMemo, useState, type FormEvent } from "react";

import { Empty, ErrorBox, Field, Title, useRows } from "@/components/shared";
import { queryKeys, useSettings } from "@/lib/queries";
import {
  ChatGptLoginCard,
  ChatGptQuotaCard,
  ChatGptWarning,
  type ChatGptLogin,
} from "./ChatGptPanel";

export type ProviderPreset = {
  id: string;
  name: string;
  category: string;
  kind: string;
  default_url: string;
  api_key_required: boolean;
  allow_private_default: boolean;
  extra_config_schema: { key: string; label: string; required?: boolean }[];
};

export const FEATURED_PRESET_IDS = [
  "openai",
  "chatgpt",
  "anthropic",
  "gemini",
  "openrouter",
  "ollama",
] as const;

export function visibleProviderFields(
  preset: ProviderPreset | null,
  isNew: boolean,
) {
  const custom = !preset || preset.id === "custom";
  return {
    picker: isNew && !preset,
    name: custom || !isNew,
    kind: custom,
    url: custom || !preset.default_url,
    apiKey: !preset || preset.api_key_required,
    extra: Boolean(preset?.extra_config_schema.length),
  };
}

const FEATURED_PRESET_ID_SET = new Set<string>(FEATURED_PRESET_IDS);

function groupPresets(items: ProviderPreset[]) {
  return Object.entries(
    items.reduce<Record<string, ProviderPreset[]>>((groups, item) => {
      (groups[item.category] ||= []).push(item);
      return groups;
    }, {}),
  );
}

function PresetChips({
  items,
  selectedId,
  onPick,
}: {
  items: ProviderPreset[];
  selectedId?: string;
  onPick: (preset: ProviderPreset) => void;
}) {
  return (
    <div className="provider-chip-row">
      {items.map((preset) => (
        <button
          key={preset.id}
          type="button"
          className={
            "provider-chip" + (selectedId === preset.id ? " is-selected" : "")
          }
          onClick={() => onPick(preset)}
        >
          {preset.name}
        </button>
      ))}
    </div>
  );
}

export function Providers() {
  const rows = useRows("/providers"),
    qc = useQueryClient();
  const setting = useSettings<{ data: { chatgpt_enabled?: boolean } }>();
  const chatgptEnabled = setting.data?.data.chatgpt_enabled === true;
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
    [selectedPreset, setSelectedPreset] = useState<ProviderPreset | null>(null),
    [name, setName] = useState(""),
    [kind, setKind] = useState("compatible"),
    [baseUrl, setBaseUrl] = useState(""),
    [allowPrivate, setAllowPrivate] = useState(false),
    [consent, setConsent] = useState(false),
    [login, setLogin] = useState<ChatGptLogin | null>(null);
  const fields = visibleProviderFields(selectedPreset, !editing);
  const featured = useMemo(
    () =>
      FEATURED_PRESET_IDS.map((id) =>
        presets.data?.find((preset) => preset.id === id),
      ).filter(
        (preset): preset is ProviderPreset =>
          Boolean(preset) && (preset!.id !== "chatgpt" || chatgptEnabled),
      ),
    [presets.data, chatgptEnabled],
  );
  const filtered = useMemo(() => {
    const needle = filter.trim().toLocaleLowerCase();
    const all = (presets.data || []).filter(
      (preset) => preset.id !== "chatgpt" || chatgptEnabled,
    );
    if (!needle) return all;
    return all.filter((preset) =>
      (preset.name + preset.category).toLocaleLowerCase().includes(needle),
    );
  }, [filter, presets.data, chatgptEnabled]);
  function applyPreset(preset: ProviderPreset | null) {
    setSelectedPreset(preset);
    setFilter("");
    setConsent(false);
    if (editing || !preset) return;
    setName(preset.id === "custom" ? "" : preset.name);
    setKind(preset.kind);
    setBaseUrl(preset.default_url || "");
    setAllowPrivate(preset.allow_private_default);
  }
  function pickPreset(preset: ProviderPreset) {
    if (!open) openForm(null);
    applyPreset(preset);
  }
  function openForm(row: Row | null) {
    setEditing(row);
    setFilter("");
    setConsent(false);
    if (row) {
      const preset =
        presets.data?.find((item) => item.id === row.data.preset_id) || null;
      setSelectedPreset(preset);
      // Editing an existing ChatGPT row already carries the user's earlier
      // consent; the server only re-prompts when switching *into* it.
      setConsent(preset?.id === "chatgpt");
      setName(row.data.name || "");
      setKind(row.data.kind || "compatible");
      setBaseUrl(row.data.base_url || "");
      setAllowPrivate(Boolean(row.data.allow_private));
    } else {
      setSelectedPreset(null);
      setName("");
      setKind("compatible");
      setBaseUrl("");
      setAllowPrivate(false);
    }
    setOpen(true);
  }
  async function startChatGptLogin(providerId: string) {
    setError(null);
    try {
      const started = await api<{
        login_id: string;
        verification_url: string;
        user_code: string;
        interval: number;
      }>("/providers/chatgpt/device/start", "POST", {
        provider_id: providerId,
      });
      setLogin({
        providerId,
        ...started,
        status: "pending",
      });
    } catch (e) {
      setError(e);
    }
  }

  const completeLogin = useCallback(
    async (_account: ChatGptLogin["account"]) => {
      const current = login;
      setLogin(null);
      if (!current) return;
      setBusy(true);
      try {
        await api(`/providers/${current.providerId}/sync-models`, "POST");
        setNotice(t("ChatGPT に接続し、モデルを取得しました。"));
      } catch (e) {
        setError(e);
      } finally {
        setBusy(false);
      }
      void qc.invalidateQueries({ queryKey: ["/providers"] });
      void qc.invalidateQueries({ queryKey: ["/models"] });
      void qc.invalidateQueries({ queryKey: ["/settings"] });
    },
    [login, qc],
  );

  const failedLogin = useCallback((message: string) => {
    setLogin(null);
    setError(new Error(message));
  }, []);

  async function cancelLogin() {
    const current = login;
    setLogin(null);
    if (!current) return;
    try {
      await api(`/providers/chatgpt/device/${current.login_id}/cancel`, "POST");
    } catch {
      // The local card is already closed; a stale server state expires in 15m.
    }
  }

  async function logout(row: Row) {
    setBusy(true);
    setError(null);
    try {
      await api(`/providers/${row.id}/logout`, "POST");
      setNotice(t("ログアウトしました"));
      qc.removeQueries({ queryKey: ["/providers", row.id, "quota"] });
      void qc.invalidateQueries({ queryKey: ["/providers"] });
      void qc.invalidateQueries({ queryKey: ["/models"] });
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    if (selectedPreset?.id === "chatgpt" && !consent) {
      setError(
        new Error(
          t(
            "ChatGPT 連携は非公式・自己責任の機能です。同意にチェックしてから保存してください。",
          ),
        ),
      );
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const result = await api<{
        id?: string;
        model_sync?: {
          status: string;
          count: number;
          auto_count: number;
          error?: string;
        };
      }>(
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
          extra_config: Object.fromEntries(
            (selectedPreset?.extra_config_schema || []).map((field) => [
              field.key,
              f.get("extra_config." + field.key) || "",
            ]),
          ),
          acknowledge_unofficial:
            selectedPreset?.id === "chatgpt" ? consent : false,
        },
      );
      setOpen(false);
      const sync = result.model_sync;
      if (sync?.status === "ok") {
        setNotice(
          `${sync.count}件のモデルを取得し、${sync.auto_count}件をAuto候補へ追加しました`,
        );
      } else if (sync?.status === "failed") {
        const missing = sync.error?.startsWith("missing_extra_config:")
          ? "追加設定が不足しています: " +
            sync.error.slice("missing_extra_config:".length)
          : "モデル取得に失敗しました。";
        setNotice(
          "Providerは保存しましたが、" +
            missing +
            " 接続を確認して「モデル取得」で再試行してください。",
        );
      }
      void qc.invalidateQueries({ queryKey: ["/providers"] });
      void qc.invalidateQueries({ queryKey: ["/models"] });
      void qc.invalidateQueries({ queryKey: ["/settings"] });
      if (selectedPreset?.id === "chatgpt" && !editing && result.id) {
        await startChatGptLogin(result.id);
      }
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  async function action(row: Row, name: string) {
    setBusy(true);
    setError(null);
    try {
      const r = await api<{ count?: number; auto_count?: number }>(
        "/providers/" + row.id + "/" + name,
        "POST",
      );
      setNotice(
        name === "test"
          ? "接続できました（推論テストは行っていません）"
          : r.count +
              "件のモデルを取得し、" +
              (r.auto_count || 0) +
              "件をAuto候補へ追加しました",
      );
      void qc.invalidateQueries({ queryKey: ["/models"] });
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  async function remove(row: Row) {
    setBusy(true);
    setError(null);
    try {
      await api("/providers/" + row.id, "DELETE");
      setNotice(t("Providerを削除しました"));
      void qc.invalidateQueries({ queryKey: ["/providers"] });
      void qc.invalidateQueries({ queryKey: ["/models"] });
      void qc.invalidateQueries({ queryKey: ["/settings"] });
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
        sub={t(
          "接続を選んでAPI Keyを保存すると、モデル一覧とAuto候補を自動設定します。",
        )}
        action={
          <Button
            onClick={() => {
              openForm(null);
            }}
          >
            <Plus size={16} />
            {t("Providerを追加")}
          </Button>
        }
      />
      <div className="notice">
        <KeyRound size={16} />
        {t("API Keyは暗号化して保存され、保存後は再表示されません。")}
      </div>
      <section className="card provider-consent">
        <label className="check">
          <input
            type="checkbox"
            checked={chatgptEnabled}
            onChange={async (event) => {
              setError(null);
              try {
                await api("/settings", "PUT", {
                  chatgpt_enabled: event.target.checked,
                });
                await qc.invalidateQueries({ queryKey: queryKeys.settings });
              } catch (cause) {
                setError(cause);
              }
            }}
          />
          {t("非公式ChatGPT連携を有効にする")}
        </label>
        <p className="provider-form-hint">
          {t(
            "Codex CLI の非公開バックエンドを使う連携です。有効にするとプリセットに表示されます。",
          )}
        </p>
      </section>
      <ErrorBox error={error || rows.error} />
      {rows.data?.some((row) => row.data.preset_id === "chatgpt") && (
        <ChatGptWarning />
      )}
      {notice && <p className="success">{notice}</p>}
      {login && (
        <ChatGptLoginCard
          login={login}
          onComplete={completeLogin}
          onFailed={failedLogin}
          onCancel={() => void cancelLogin()}
        />
      )}
      {open && fields.picker && (
        <div className="card provider-picker">
          <div>
            <h3>{t("接続を選ぶ")}</h3>
            <p>{t("よく使う接続から選ぶか、名前で検索してください。")}</p>
          </div>
          <input
            type="search"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={t("Providerを検索")}
            aria-label={t("Providerを検索")}
          />
          {!filter && featured.length > 0 && (
            <div>
              <h4>{t("よく使う接続")}</h4>
              <PresetChips items={featured} onPick={pickPreset} />
            </div>
          )}
          <div className="provider-preset-groups">
            {groupPresets(
              filter
                ? filtered
                : filtered.filter(
                    (preset) => !FEATURED_PRESET_ID_SET.has(preset.id),
                  ),
            ).map(([category, items]) => (
              <div key={category}>
                <h4>{t(category)}</h4>
                <PresetChips items={items} onPick={pickPreset} />
              </div>
            ))}
          </div>
          <div className="form-actions">
            <Button
              type="button"
              variant="ghost"
              onClick={() => setOpen(false)}
            >
              {t("キャンセル")}
            </Button>
          </div>
        </div>
      )}
      {open && !fields.picker && (
        <form
          className="card form-grid provider-form-simple"
          key={editing?.id || selectedPreset?.id || "new"}
          onSubmit={submit}
        >
          <input
            type="hidden"
            name="preset_id"
            value={selectedPreset?.id || ""}
          />
          {!fields.kind && <input type="hidden" name="kind" value={kind} />}
          {!fields.name && <input type="hidden" name="name" value={name} />}
          {!fields.url && (
            <input type="hidden" name="base_url" value={baseUrl} />
          )}
          <div className="provider-selected">
            <div>
              <small>{t("選択中:")}</small>
              <strong>
                {selectedPreset?.name || editing?.data.name || t("カスタム")}
              </strong>
            </div>
            {!editing && (
              <Button
                type="button"
                variant="ghost"
                onClick={() => applyPreset(null)}
              >
                {t("Providerを変更")}
              </Button>
            )}
          </div>
          {selectedPreset?.id === "chatgpt" && (
            <>
              <ChatGptWarning />
              <label className="check provider-consent">
                <input
                  type="checkbox"
                  checked={consent}
                  required
                  onChange={(e) => setConsent(e.target.checked)}
                />
                {t("上記を理解し、非公式・自己責任であることに同意します")}
              </label>
            </>
          )}
          <p className="provider-form-hint">
            {selectedPreset?.id === "chatgpt"
              ? t(
                  "保存後に ChatGPT のログイン（デバイスコード）を開始します。API Key は使いません。",
                )
              : selectedPreset && !selectedPreset.api_key_required
                ? t(
                    "ローカル接続はAPI Key不要です。保存するとモデルを取得します。",
                  )
                : selectedPreset?.default_url
                  ? t("API Keyを貼り付けて保存すると、モデルを自動取得します。")
                  : t("接続先URLを入力してください。")}
          </p>
          {fields.name && (
            <Field label={t("表示名")}>
              <input
                name="name"
                required
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="My OpenAI"
              />
            </Field>
          )}
          {fields.kind && (
            <Field
              label={t("カスタム接続方式")}
              hint={t(
                "カスタム以外では選択したプリセットの方式が使用されます。",
              )}
            >
              <select
                name="kind"
                value={kind}
                onChange={(e) => setKind(e.target.value)}
              >
                <option value="compatible">{t("OpenAI互換")}</option>
                <option value="anthropic">Anthropic Messages</option>
                <option value="gemini">Gemini GenerateContent</option>
                <option value="openai">
                  {t("OpenAI Responses（従来設定）")}
                </option>
                <option value="openrouter">
                  {t("OpenRouter（従来設定）")}
                </option>
                <option value="ollama">{t("Ollama（従来設定）")}</option>
                <option value="lmstudio">{t("LM Studio（従来設定）")}</option>
              </select>
            </Field>
          )}
          {fields.url && (
            <Field
              label="Base URL"
              hint={t("カスタムとURL未設定のプリセットでは必須です。")}
            >
              <input
                name="base_url"
                required
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="https://api.example.com/v1"
              />
            </Field>
          )}
          {fields.apiKey && (
            <Field
              label="API Key"
              hint={
                editing
                  ? t("空欄なら現在のKeyを保持します。")
                  : t("接続方式とURLは自動で設定されます。")
              }
            >
              <input
                name="api_key"
                type="password"
                autoComplete="off"
                required={!editing}
              />
            </Field>
          )}
          {selectedPreset?.extra_config_schema.map((field) => (
            <Field
              key={field.key}
              label={field.label}
              hint={t("このProvider固有の接続設定です。")}
            >
              <input
                name={"extra_config." + field.key}
                required={field.required}
                defaultValue={editing?.data.extra_config?.[field.key] || ""}
              />
            </Field>
          ))}
          <details className="provider-advanced">
            <summary>{t("表示名やURLを変更")}</summary>
            {!fields.name && (
              <Field label={t("表示名")}>
                <input value={name} onChange={(e) => setName(e.target.value)} />
              </Field>
            )}
            {!fields.url && (
              <Field label="Base URL">
                <input
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                />
              </Field>
            )}
            <Field
              label={t("要求回数の上限")}
              hint={t("Providerへの要求を制限します。0は無制限です。")}
            >
              <div className="rate-limit-input">
                <input
                  name="rate_limit_rpm"
                  type="number"
                  min="0"
                  max="10000"
                  step="1"
                  defaultValue={editing?.data.rate_limit_rpm || 0}
                />
                <select
                  name="rate_limit_period"
                  defaultValue={editing?.data.rate_limit_period || "minute"}
                >
                  <option value="minute">{t("回 / 分")}</option>
                  <option value="second">{t("回 / 秒")}</option>
                </select>
              </div>
            </Field>
            <label className="check">
              <input
                type="checkbox"
                name="allow_private"
                checked={allowPrivate}
                onChange={(e) => setAllowPrivate(e.target.checked)}
              />
              {t("明示したLAN・ローカル接続先を許可")}
            </label>
          </details>
          <div className="form-actions">
            <Button disabled={busy}>{t("保存")}</Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setOpen(false)}
            >
              {t("キャンセル")}
            </Button>
          </div>
        </form>
      )}
      {rows.data?.map((row) => {
        const preset = presets.data?.find(
          (item) => item.id === row.data.preset_id,
        );
        const chatgptConnected =
          row.data.preset_id === "chatgpt" &&
          (row.data.status === "ready" ||
            (row.data.status == null && Boolean(row.data.auth)));
        return (
          <div className="provider-block" key={row.id}>
            <div className="card provider-card">
              <div className="provider-icon">
                <Server size={22} />
              </div>
              <div className="grow">
                <h3>{row.data.name}</h3>
                <p>
                  {preset?.name || row.data.kind}
                  <span> · </span>
                  {row.data.preset_id === "chatgpt" ? (
                    chatgptConnected ? (
                      <>
                        {row.data.auth?.email || t("ログイン済み")}
                        {row.data.auth?.plan_type
                          ? ` · ${row.data.auth.plan_type}`
                          : ""}
                      </>
                    ) : (
                      t("未ログイン")
                    )
                  ) : row.data.has_secret_id ? (
                    t("API Key 保存済み")
                  ) : (
                    t("API Key なし")
                  )}
                </p>
                <small>{row.data.base_url}</small>
              </div>
              <div className="row-actions">
                {row.data.preset_id === "chatgpt" &&
                !chatgptConnected &&
                chatgptEnabled ? (
                  <Button
                    variant="outline"
                    disabled={busy}
                    onClick={() => void startChatGptLogin(row.id)}
                  >
                    {t("ChatGPT でログイン")}
                  </Button>
                ) : null}
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => action(row, "test")}
                >
                  {t("接続テスト")}
                </Button>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => action(row, "sync-models")}
                >
                  <RefreshCw size={14} />
                  {t("モデル取得")}
                </Button>
                <Button
                  variant="ghost"
                  onClick={() => {
                    openForm(row);
                  }}
                >
                  {t("編集")}
                </Button>
                <ConfirmButton
                  variant="ghost"
                  disabled={busy}
                  title={t("Providerを削除")}
                  message={t(
                    "Providerと取得済みのモデルを削除しますか？ 元に戻せません。",
                  )}
                  confirmLabel={t("削除")}
                  onConfirm={() => remove(row)}
                >
                  {t("削除")}
                </ConfirmButton>
              </div>
            </div>
            {row.data.preset_id === "chatgpt" ? (
              <ChatGptQuotaCard
                row={row}
                loginEnabled={chatgptEnabled}
                onLogin={() => void startChatGptLogin(row.id)}
                onLogout={() => void logout(row)}
              />
            ) : null}
          </div>
        );
      })}
      {!rows.data?.length && !open && (
        <div className="provider-empty">
          <Empty>{t("よく使う接続を選んで、チャットを始めましょう。")}</Empty>
          {featured.length > 0 && (
            <div className="provider-quick-add">
              <h4>{t("よく使う接続")}</h4>
              <PresetChips items={featured} onPick={pickPreset} />
              <Button
                type="button"
                variant="ghost"
                onClick={() => openForm(null)}
              >
                {t("その他のProvider")}
              </Button>
            </div>
          )}
        </div>
      )}
    </>
  );
}
