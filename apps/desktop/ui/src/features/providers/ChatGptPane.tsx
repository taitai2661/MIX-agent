/// Dedicated settings surface for the unofficial ChatGPT / Codex integration.
///
/// This is deliberately not a provider preset: there is nothing to configure
/// (base URL is pinned, there is no API key), the login is a device flow, and
/// there must be exactly one connection. Keeping it out of the generic provider
/// form prevents a second row from being created with a different URL or a
/// missing acknowledgement.

import { useCallback, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  ExternalLink,
  KeyRound,
  LogOut,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { MODELS_QUERY_KEY, PROVIDERS_QUERY_KEY } from "@/lib/queries";
import { useApplySettings, useSettingsForm } from "@/lib/settings";
import { useUi } from "@/lib/ui";
import type { ChatGptStatus, ProviderView } from "@/lib/types";
import { Card, Row, Section } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Spinner } from "@/components/ui/spinner";
import { useConfirm } from "@/components/ui/dialog";
import { ErrorBox } from "@/components/shared";
import {
  ChatGptLoginCard,
  ChatGptQuotaCard,
  ChatGptWarning,
  type ChatGptLogin,
} from "./ChatGptPanel";

const STATUS_QUERY_KEY = ["chatgpt_status"] as const;

function connected(provider: ProviderView): boolean {
  return (
    provider.status === "ready" ||
    (provider.status == null &&
      Boolean(provider.auth?.account_id || provider.auth?.email))
  );
}

export function ChatGptPane() {
  const queryClient = useQueryClient();
  const { data: settings, loaded: settingsLoaded } = useSettingsForm();
  const applySettings = useApplySettings();
  const openSettings = useUi((s) => s.openSettings);
  const { confirm, dialog } = useConfirm();
  const enabled = settings.chatgpt_enabled === true;
  const [login, setLogin] = useState<ChatGptLogin | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [synced, setSynced] = useState<number | null>(null);
  const [savingEnabled, setSavingEnabled] = useState(false);

  const status = useQuery<ChatGptStatus>({
    queryKey: STATUS_QUERY_KEY,
    queryFn: api.chatgptStatus,
  });
  const provider = status.data?.provider ?? null;

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: STATUS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
  }, [queryClient]);

  const startLogin = useCallback(
    async (providerId: string) => {
      setError(null);
      setSynced(null);
      if (login) {
        try {
          await api.chatgptDeviceCancel(login.login_id);
        } catch {
          // The backend expires it on its own.
        }
      }
      try {
        const started = await api.chatgptDeviceStart(providerId);
        setLogin({ ...started, providerId, status: "pending" });
      } catch (err) {
        setError(errorMessage(err));
      }
    },
    [login],
  );

  const connect = useMutation({
    mutationFn: api.chatgptEnsure,
    onSuccess: (created) => {
      invalidate();
      void startLogin(created.id);
    },
    onError: (err) => setError(errorMessage(err)),
  });

  const syncModels = useMutation({
    mutationFn: (providerId: string) => api.connectProvider(providerId),
    onSuccess: (count) => {
      setSynced(count);
      invalidate();
    },
    onError: (err) => setError(errorMessage(err)),
  });

  const completeLogin = useCallback(
    async (_account: ChatGptLogin["account"]) => {
      const current = login;
      setLogin(null);
      if (!current) return;
      try {
        const count = await api.connectProvider(current.providerId);
        setSynced(count);
      } catch (err) {
        // Tokens are already stored; surface the model-discovery failure but
        // leave the connection usable.
        setError(errorMessage(err));
      } finally {
        invalidate();
      }
    },
    [invalidate, login],
  );

  const cancelLogin = async () => {
    const current = login;
    setLogin(null);
    if (!current) return;
    try {
      await api.chatgptDeviceCancel(current.login_id);
    } catch {
      // A local close is enough; the code expires server-side.
    }
  };

  const logout = async () => {
    if (!provider) return;
    const ok = await confirm({
      title: t("ChatGPT からログアウトしますか？"),
      description: t(
        "保存した認証情報と取得済みのモデルをこの PC から削除します。再度使うにはログインし直してください。",
      ),
      confirmLabel: t("ログアウト"),
      tone: "danger",
    });
    if (!ok) return;
    setError(null);
    try {
      await api.chatgptLogout(provider.id);
      queryClient.removeQueries({ queryKey: ["chatgpt_quota", provider.id] });
      setSynced(null);
      invalidate();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <Section>
      <Card title={t("ChatGPT（非公式・専用連携）")} icon={<Bot size={13} />}>
        <ChatGptWarning />
        <p className="m-0 text-xs text-muted">
          {t(
            "OpenAI API キー方式のプロバイダーとは別の専用連携です。プロバイダー一覧には表示されず、接続したモデルだけがモデルタブに並びます。",
          )}
        </p>
        <label className="flex items-start gap-2 text-xs text-muted">
          <Switch
            checked={enabled}
            disabled={!settingsLoaded || savingEnabled}
            onCheckedChange={(next) => {
              setSavingEnabled(true);
              void applySettings({ chatgpt_enabled: next }).finally(() => {
                setSavingEnabled(false);
                if (!next) {
                  const current = login;
                  setLogin(null);
                  if (current) {
                    void api
                      .chatgptDeviceCancel(current.login_id)
                      .catch(() => undefined);
                  }
                }
              });
            }}
            aria-label={t("非公式ChatGPT連携を有効にする")}
          />
          <span>{t("非公式ChatGPT連携を有効にする")}</span>
        </label>
      </Card>

      {error && <ErrorBox error={error} />}
      {status.isError && <ErrorBox error={status.error} />}

      {status.isPending ? (
        <Card>
          <div className="flex items-center gap-2 text-xs text-muted">
            <Spinner size={12} /> {t("読み込み中…")}
          </div>
        </Card>
      ) : !provider ? (
        <Card title={t("まだ接続されていません")} icon={<KeyRound size={13} />}>
          <div className="flex flex-col gap-2 text-xs text-muted">
            <p className="m-0">
              {t(
                "下のボタンからデバイスコードでログインします。ブラウザでコードを入力するだけで、パスワードをこのアプリに渡す必要はありません。",
              )}
            </p>
            <Row>
              <Button
                variant="primary"
                disabled={
                  !settingsLoaded ||
                  !enabled ||
                  savingEnabled ||
                  connect.isPending
                }
                onClick={() => connect.mutate()}
              >
                {connect.isPending ? (
                  <Spinner size={12} />
                ) : (
                  <ExternalLink size={12} />
                )}
                {t("ChatGPT を接続")}
              </Button>
              {!enabled && (
                <span className="text-2xs text-faint">
                  {t("先に上のスイッチで有効にしてください。")}
                </span>
              )}
            </Row>
          </div>
        </Card>
      ) : (
        <>
          <Card title={t("接続状況")} icon={<Bot size={13} />}>
            <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted">
              <Badge
                tone={connected(provider) ? "success" : "warning"}
                size="sm"
              >
                {connected(provider) ? t("ログイン済み") : t("未ログイン")}
              </Badge>
              {provider.auth?.email && <span>{provider.auth.email}</span>}
              {provider.auth?.plan_type && (
                <Badge size="sm">{provider.auth.plan_type}</Badge>
              )}
              <Badge size="sm">{t("サブスクリプション")}</Badge>
              <span className="text-faint">
                {tf("{n} モデル", { n: status.data?.models ?? 0 })}
              </span>
              {synced !== null && (
                <Badge tone="success" size="sm">
                  {tf("{n} 件を取り込みました", { n: synced })}
                </Badge>
              )}
            </div>
            <Row align="between" className="gap-2">
              <span className="flex flex-wrap items-center gap-1.5">
                {!connected(provider) ? (
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={!enabled || savingEnabled}
                    onClick={() => void startLogin(provider.id)}
                  >
                    <ExternalLink size={12} />
                    {t("ChatGPT でログイン")}
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    disabled={syncModels.isPending}
                    onClick={() => syncModels.mutate(provider.id)}
                  >
                    {syncModels.isPending ? (
                      <Spinner size={12} />
                    ) : (
                      <RefreshCw size={12} />
                    )}
                    {t("モデルを再取得")}
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => openSettings("models")}
                >
                  {t("モデルを表示")}
                </Button>
              </span>
              {connected(provider) && (
                <Button size="sm" variant="ghost" onClick={() => void logout()}>
                  <LogOut size={12} />
                  {t("ログアウト")}
                </Button>
              )}
            </Row>
            {!enabled && (
              <p className="m-0 text-xs text-faint">
                {t(
                  "連携は無効です。上のスイッチを有効にすると、この接続を再び使えます。",
                )}
              </p>
            )}
          </Card>

          {connected(provider) && (
            <ChatGptQuotaCard
              provider={provider}
              loginEnabled={enabled}
              onLogin={() => void startLogin(provider.id)}
              onLogout={() => void logout()}
            />
          )}
        </>
      )}

      {login && (
        <ChatGptLoginCard
          login={login}
          onComplete={completeLogin}
          onFailed={(message) => {
            setError(message);
            setLogin(null);
          }}
          onCancel={() => void cancelLogin()}
        />
      )}

      {provider && !enabled && (
        <Card title={t("無効化中")} icon={<ShieldAlert size={13} />}>
          <p className="m-0 text-xs text-muted">
            {t(
              "接続情報は残っていますが、エージェントからは使用できません。有効に戻すとそのまま利用できます。",
            )}
          </p>
        </Card>
      )}

      {dialog}
    </Section>
  );
}
