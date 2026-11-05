import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink, LogOut, RefreshCw, ShieldAlert } from "lucide-react";

import { api, HttpError } from "@/app/api";
import { t } from "@/app/i18n";
import { Button } from "@/components/button";
import { resetTimeLabel } from "./quota";

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

/** Map the device-flow status codes to user-facing text. */
function loginErrorText(code: string): string {
  switch (code) {
    case "chatgpt_login_expired":
      return t(
        "ログインの有効期限が切れました。もう一度ログインしてください。",
      );
    case "chatgpt_login_not_found":
      return t("ログイン情報が見つかりません。もう一度ログインしてください。");
    case "chatgpt_login_failed":
    case "chatgpt_device_failed":
      return t("ChatGPT のログインに失敗しました。");
    case "chatgpt_device_unavailable":
      return t(
        "このアカウントでは ChatGPT のデバイスコードログインを利用できません。",
      );
    default:
      return code;
  }
}

/** Auth failures need a fresh login; a network/5xx failure only needs a retry. */
function isAuthQuotaError(error: unknown): boolean {
  return error instanceof HttpError && error.status === 401;
}

function quotaErrorText(error: unknown): string {
  const text = errorText(error);
  if (text.includes("chatgpt_auth_required")) {
    return t("ChatGPT の再ログインが必要です。");
  }
  if (text.includes("chatgpt_auth_expired")) {
    return t(
      "ChatGPT のログインの有効期限が切れました。もう一度ログインしてください。",
    );
  }
  if (
    text.includes("chatgpt_quota_failed") ||
    text.includes("chatgpt_quota_unavailable")
  ) {
    return t("使用枠を取得できませんでした。");
  }
  if (text.includes("chatgpt_rate_limited")) {
    return t("ChatGPT の使用枠に達しています。");
  }
  if (text.includes("chatgpt_refresh_failed")) {
    return t("ChatGPT の認証更新に失敗しました。");
  }
  if (text.includes("chatgpt_credentials_save_failed")) {
    return t("ChatGPT の認証情報を保存できませんでした。");
  }
  return text;
}

const STALE_AFTER_MS = 15 * 60 * 1000;

function isStale(quota: Quota): boolean {
  const captured = Date.parse(quota.captured_at);
  return Number.isFinite(captured) && Date.now() - captured > STALE_AFTER_MS;
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
          failedRef.current(
            loginErrorText(status.error || "chatgpt_login_failed"),
          );
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

export function ChatGptQuotaCard({
  row,
  loginEnabled,
  onLogin,
  onLogout,
}: {
  row: { id: string; data: Record<string, unknown> };
  loginEnabled: boolean;
  onLogin: () => void;
  onLogout: () => void;
}) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const auth = row.data.auth as
    | { email?: string; plan_type?: string; account_id?: string }
    | undefined;
  // `status` is derived from the stored credential blob server-side; the
  // account id is only a display claim and may be absent.
  const connected =
    row.data.status === "ready" || (row.data.status == null && Boolean(auth));
  const quota = useQuery<Quota>({
    queryKey: ["/providers", row.id, "quota"],
    queryFn: () => api(`/providers/${row.id}/quota`),
    enabled: connected,
    // Inference headers and SSE events update this snapshot immediately; the
    // periodic poll is only a safety net.
    refetchInterval: 300_000,
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
      // The adapter persists a rejected refresh token as auth_failed; refresh
      // the provider row so the card stops claiming the account is connected.
      void qc.invalidateQueries({ queryKey: ["/providers"] });
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
          ) : loginEnabled ? (
            <Button type="button" disabled={busy} onClick={onLogin}>
              {t("ChatGPT でログイン")}
            </Button>
          ) : (
            <span className="provider-quota-hint">
              {t("非公式連携が無効です")}
            </span>
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
          <p>{quotaErrorText(quota.error)}</p>
          {isAuthQuotaError(quota.error) ? (
            <Button type="button" variant="outline" onClick={onLogin}>
              {t("再ログイン")}
            </Button>
          ) : null}
        </div>
      ) : quota.data ? (
        <>
          {isStale(quota.data) ? (
            <p className="provider-quota-hint">
              {t("15 分以上前の情報です。")}
            </p>
          ) : null}
          <div className="quota-grid">
            {quota.data.windows.map((window, index) => (
              <div className="quota-row" key={`${window.id}-${index}`}>
                <div className="quota-label">
                  <span>{windowLabel(window)}</span>
                  <strong>
                    {Math.max(0, 100 - Math.round(window.used_percent))}%{" "}
                    {t("残り")}
                  </strong>
                </div>
                <div
                  className="quota-bar"
                  role="progressbar"
                  aria-label={windowLabel(window)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(window.used_percent)}
                  aria-valuetext={`${Math.round(window.used_percent)}% ${t("使用済み")}`}
                >
                  <span
                    style={{ width: `${Math.min(100, window.used_percent)}%` }}
                  />
                </div>
                {window.resets_at ? (
                  <small>
                    {t("リセット:")} {resetTimeLabel(window.resets_at)}
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
              <br />
              {t(
                "リセットを待つか、APIキー方式のProviderに切り替えてください。",
              )}
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
        <br />
        {t("リセット時刻は端末のタイムゾーンで表示し、UTC も併記しています。")}
      </p>
    </div>
  );
}
