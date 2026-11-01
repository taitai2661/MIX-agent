import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink, LogOut, RefreshCw, ShieldAlert } from "lucide-react";

import { api } from "@/app/api";
import { t } from "@/app/i18n";
import { Button } from "@/components/button";

export type ChatGptLogin = {
  providerId: string;
  login_id: string;
  verification_url: string;
  user_code: string;
  interval: number;
  status: "pending" | "complete" | "error" | "expired";
  error?: string | null;
  account?: { email?: string; plan_type?: string } | null;
};

export type QuotaWindow = {
  id: string;
  used_percent: number;
  window_minutes: number;
  window_seconds: number;
  resets_at: number;
};

export type Quota = {
  plan_type: string;
  captured_at: string;
  windows: QuotaWindow[];
  credits: {
    has_credits: boolean;
    unlimited: boolean;
    balance: string;
  } | null;
  limit_reached: boolean;
};

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function ChatGptWarning() {
  return (
    <div className="provider-warning" role="note">
      <ShieldAlert size={18} aria-hidden="true" />
      <div>
        <strong>{t("非公式・自己責任の機能です")}</strong>
        <p>
          {t(
            "この連携は OpenAI の公式 API ではなく、Codex CLI が使う非公開バックエンドを利用します。利用規約・アカウント制限に抵触する可能性、予告なく停止する可能性があります。MIX-agent は動作や継続利用を保証しません。",
          )}
        </p>
      </div>
    </div>
  );
}

export function ChatGptLoginCard({
  login,
  onComplete,
  onFailed,
  onCancel,
}: {
  login: ChatGptLogin;
  onComplete: (account: ChatGptLogin["account"]) => void;
  onFailed: (message: string) => void;
  onCancel: () => void;
}) {
  const completeRef = useRef(onComplete);
  const failedRef = useRef(onFailed);
  const cancelRef = useRef(onCancel);
  completeRef.current = onComplete;
  failedRef.current = onFailed;
  cancelRef.current = onCancel;
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (login.status !== "pending") return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const delay = Math.max(2, login.interval || 2) * 1000;
    const poll = async () => {
      try {
        const status = await api<{
          status: string;
          error?: string | null;
          account?: ChatGptLogin["account"];
        }>(`/providers/chatgpt/device/${login.login_id}`);
        if (cancelled) return;
        if (status.status === "complete") {
          completeRef.current(status.account ?? null);
          return;
        }
        if (status.status === "error" || status.status === "expired") {
          failedRef.current(status.error || "chatgpt_login_failed");
          return;
        }
      } catch (error) {
        if (cancelled) return;
        failedRef.current(errorText(error));
        return;
      }
      if (!cancelled) timer = setTimeout(poll, delay);
    };
    timer = setTimeout(poll, delay);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [login.login_id, login.interval, login.status]);

  async function copyCode() {
    try {
      await navigator.clipboard.writeText(login.user_code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="card provider-login-card">
      <h3>{t("ChatGPT にログイン")}</h3>
      <p>
        {t(
          "表示された URL を開き、ワンタイムコードを入力してください。この画面は自動で完了を確認します。",
        )}
      </p>
      <div className="provider-login-code">
        <code>{login.user_code}</code>
        <Button type="button" variant="outline" onClick={copyCode}>
          {copied ? t("コピーしました") : t("コードをコピー")}
        </Button>
      </div>
      <Button asChild variant="outline">
        <a href={login.verification_url} target="_blank" rel="noreferrer">
          <ExternalLink size={14} />
          {t("認証ページを開く")}
        </a>
      </Button>
      <p className="provider-login-status" aria-live="polite">
        {login.status === "pending"
          ? t("認証の完了を待っています…")
          : login.status}
      </p>
      <div className="form-actions">
        <Button
          type="button"
          variant="ghost"
          onClick={() => cancelRef.current()}
        >
          {t("キャンセル")}
        </Button>
      </div>
    </div>
  );
}

function windowLabel(window: QuotaWindow): string {
  if (window.id === "5h") return t("5時間枠");
  if (window.id === "week") return t("週次枠");
  return window.window_minutes
    ? `${Math.round(window.window_minutes / 60)}h`
    : t("使用枠");
}

function resetLabel(seconds: number): string {
  if (!seconds) return "";
  return new Date(seconds * 1000).toLocaleString();
}

export function ChatGptQuotaCard({
  row,
  onLogin,
  onLogout,
}: {
  row: { id: string; data: Record<string, unknown> };
  onLogin: () => void;
  onLogout: () => void;
}) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const auth = row.data.auth as
    | { email?: string; plan_type?: string; account_id?: string }
    | undefined;
  const connected = Boolean(auth?.account_id);
  const quota = useQuery<Quota>({
    queryKey: ["/providers", row.id, "quota"],
    queryFn: () => api(`/providers/${row.id}/quota`),
    enabled: connected,
    refetchInterval: 60_000,
    retry: false,
  });

  async function refresh() {
    setBusy(true);
    setActionError(null);
    try {
      await api(`/providers/${row.id}/quota/refresh`, "POST");
      await qc.invalidateQueries({ queryKey: ["/providers", row.id, "quota"] });
    } catch (error) {
      setActionError(errorText(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card provider-quota-card">
      <div className="provider-quota-head">
        <div>
          <h3>{t("ChatGPT 使用枠")}</h3>
          <p>
            {connected
              ? [auth?.email, auth?.plan_type].filter(Boolean).join(" · ")
              : t("未ログイン")}
          </p>
        </div>
        <div className="row-actions">
          <Button
            type="button"
            variant="outline"
            disabled={busy || !connected}
            onClick={() => void refresh()}
          >
            <RefreshCw size={14} />
            {t("残量を更新")}
          </Button>
          {connected ? (
            <Button
              type="button"
              variant="ghost"
              disabled={busy}
              onClick={onLogout}
            >
              <LogOut size={14} />
              {t("ログアウト")}
            </Button>
          ) : (
            <Button type="button" disabled={busy} onClick={onLogin}>
              {t("ChatGPT でログイン")}
            </Button>
          )}
        </div>
      </div>
      {!connected ? (
        <p className="provider-quota-hint">
          {t("ログインすると 5 時間枠・週次枠の残量を表示します。")}
        </p>
      ) : quota.isLoading ? (
        <p>{t("残量を取得しています…")}</p>
      ) : quota.error ? (
        <div className="provider-quota-error">
          <p>{errorText(quota.error)}</p>
          <Button type="button" variant="outline" onClick={onLogin}>
            {t("再ログイン")}
          </Button>
        </div>
      ) : quota.data ? (
        <>
          <div className="quota-grid">
            {quota.data.windows.map((window) => (
              <div className="quota-row" key={window.id}>
                <div className="quota-label">
                  <span>{windowLabel(window)}</span>
                  <strong>{Math.round(window.used_percent)}%</strong>
                </div>
                <div
                  className="quota-bar"
                  role="progressbar"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(window.used_percent)}
                >
                  <span
                    style={{ width: `${Math.min(100, window.used_percent)}%` }}
                  />
                </div>
                {window.resets_at ? (
                  <small>
                    {t("リセット:")} {resetLabel(window.resets_at)}
                  </small>
                ) : null}
              </div>
            ))}
          </div>
          {quota.data.credits ? (
            <p className="provider-quota-credits">
              {quota.data.credits.unlimited
                ? t("クレジット: 無制限")
                : quota.data.credits.has_credits
                  ? `${t("クレジット残高:")} ${quota.data.credits.balance || "—"}`
                  : t("クレジットなし")}
            </p>
          ) : null}
          {quota.data.limit_reached ? (
            <p className="provider-quota-error">
              {t("使用枠に達しています。")}
            </p>
          ) : null}
          <small className="provider-quota-captured">
            {t("取得:")} {new Date(quota.data.captured_at).toLocaleTimeString()}
          </small>
        </>
      ) : null}
      {actionError ? (
        <p className="provider-quota-error" role="alert">
          {actionError}
        </p>
      ) : null}
      <p className="provider-quota-hint">
        {t(
          "残量は OpenAI の非公開エンドポイントから取得しています。表示は保証されません。",
        )}
      </p>
    </div>
  );
}
