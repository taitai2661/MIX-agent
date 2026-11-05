import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink, LogOut, RefreshCw, ShieldAlert } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { resetTime } from "@/lib/format";
import { t, tf } from "@/lib/i18n";
import { PROVIDERS_QUERY_KEY } from "@/lib/queries";
import type {
  ChatGptDeviceStart,
  ProviderView,
  QuotaSnapshot,
} from "@/lib/types";
import { Card, Row } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";

export type ChatGptLogin = ChatGptDeviceStart & {
  providerId: string;
  status: "pending" | "complete" | "error" | "expired";
  error?: string | null;
  account?: { email?: string; plan_type?: string; account_id?: string } | null;
};

export function ChatGptWarning({ compact = false }: { compact?: boolean }) {
  return (
    <div
      className="flex items-start gap-2.5 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2.5 text-xs leading-relaxed"
      role="note"
    >
      <ShieldAlert size={15} className="mt-0.5 shrink-0 text-warning" />
      <div>
        <strong className="block text-fg">
          {t("非公式・自己責任の機能です")}
        </strong>
        {!compact && (
          <p className="mt-0.5 text-muted">
            {t(
              "この連携は OpenAI の公式 API ではなく、Codex CLI が使う非公開バックエンドを利用します。利用規約・アカウント制限に抵触する可能性、予告なく停止する可能性があります。MIX-agent は動作や継続利用を保証しません。",
            )}
          </p>
        )}
      </div>
    </div>
  );
}

function windowLabel(id: string, minutes: number): string {
  if (id === "5h") return t("5時間枠");
  if (id === "week") return t("週次枠");
  return minutes ? `${Math.round(minutes / 60)}h` : t("使用枠");
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

function rawErrorText(error: unknown): string {
  if (error == null) return "";
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Auth failures need a fresh login; a network/5xx failure only needs a retry. */
function isAuthQuotaError(error: unknown): boolean {
  const text = rawErrorText(error);
  return (
    text.includes("chatgpt_auth_required") ||
    text.includes("chatgpt_auth_expired")
  );
}

const STALE_AFTER_SECONDS = 15 * 60;

function isStale(snapshot: QuotaSnapshot): boolean {
  if (!snapshot.captured_at) return false;
  return Date.now() / 1000 - snapshot.captured_at > STALE_AFTER_SECONDS;
}

export function ChatGptQuotaCard({
  provider,
  loginEnabled,
  onLogin,
  onLogout,
}: {
  provider: ProviderView;
  loginEnabled: boolean;
  onLogin: () => void;
  onLogout: () => void;
}) {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  // The credential blob is authoritative; `account_id` is only a display
  // claim and may be absent when the ID token omitted it.
  const connected =
    provider.status === "ready" ||
    (provider.status == null &&
      Boolean(provider.auth?.account_id || provider.auth?.email));
  const quota = useQuery<QuotaSnapshot | null>({
    queryKey: ["chatgpt_quota", provider.id],
    queryFn: () => api.chatgptQuota(provider.id),
    enabled: connected,
    // Inference headers and SSE events update this snapshot immediately, so
    // the periodic poll is only a safety net. One minute per open window was
    // more traffic than the private endpoint needs.
    refetchInterval: 300_000,
    retry: false,
  });

  // A permanent refresh failure is recorded in the credential blob by the
  // adapter. Re-read the provider row so the badge changes to "sign in again"
  // instead of continuing to look connected.
  useEffect(() => {
    if (quota.error) {
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
    }
  }, [quota.error, queryClient]);

  async function refresh() {
    setBusy(true);
    setActionError(null);
    try {
      await api.chatgptQuota(provider.id, true);
      await queryClient.invalidateQueries({
        queryKey: ["chatgpt_quota", provider.id],
      });
    } catch (error) {
      setActionError(errorMessage(error));
      // A rejected token is persisted as auth_failed by the adapter; refresh
      // the row so the badge switches to "sign in again" without waiting for
      // another quota poll.
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card
      title={t("ChatGPT 使用枠")}
      icon={<RefreshCw size={13} />}
      action={
        <Row className="gap-1.5">
          <Button
            variant="ghost"
            size="sm"
            disabled={busy || !connected}
            onClick={() => void refresh()}
          >
            {busy ? <Spinner size={11} /> : <RefreshCw size={12} />}
            {t("残量を更新")}
          </Button>
          {connected ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={onLogout}
            >
              <LogOut size={12} />
              {t("ログアウト")}
            </Button>
          ) : loginEnabled ? (
            <Button size="sm" onClick={onLogin}>
              {t("ChatGPT でログイン")}
            </Button>
          ) : (
            <span className="text-2xs text-faint">
              {t("非公式連携が無効です")}
            </span>
          )}
        </Row>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted">
          <span>{provider.auth?.email || t("未ログイン")}</span>
          {provider.auth?.plan_type ? (
            <Badge size="sm">{provider.auth.plan_type}</Badge>
          ) : null}
        </div>
        {!connected ? (
          <p className="text-xs text-faint">
            {t("ログインすると 5 時間枠・週次枠の残量を表示します。")}
          </p>
        ) : quota.isPending ? (
          <p className="text-xs text-faint">{t("残量を取得しています…")}</p>
        ) : quota.error ? (
          <div className="flex flex-wrap items-center gap-2 text-xs text-danger">
            <span>{errorMessage(quota.error)}</span>
            {isAuthQuotaError(quota.error) ? (
              <Button variant="default" size="sm" onClick={onLogin}>
                {t("再ログイン")}
              </Button>
            ) : null}
          </div>
        ) : quota.data ? (
          <>
            {isStale(quota.data) ? (
              <p className="text-xs text-warning">
                {t("15 分以上前の情報です。")}
              </p>
            ) : null}
            {quota.data.windows.length === 0 ? (
              <p className="text-xs text-faint">
                {t("使用枠を取得できませんでした。")}
              </p>
            ) : null}
            {quota.data.windows.map((window, index) => (
              <div
                key={`${window.id}-${index}`}
                className="flex flex-col gap-1"
              >
                <div className="flex items-baseline justify-between gap-3 text-xs">
                  <span className="text-muted">
                    {windowLabel(window.id, window.window_minutes)}
                  </span>
                  <strong className="font-mono tabular-nums text-fg">
                    {tf("残り {percent}%", {
                      percent: Math.max(
                        0,
                        100 - Math.round(window.used_percent),
                      ),
                    })}
                  </strong>
                </div>
                <div
                  className="h-1.5 overflow-hidden rounded-full bg-line"
                  role="progressbar"
                  aria-label={windowLabel(window.id, window.window_minutes)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(window.used_percent)}
                  aria-valuetext={tf("使用済み {percent}%", {
                    percent: Math.round(window.used_percent),
                  })}
                >
                  <span
                    className="block h-full rounded-full bg-brand transition-[width]"
                    style={{
                      width: `${Math.min(100, window.used_percent)}%`,
                    }}
                  />
                </div>
                {window.resets_at ? (
                  <small className="text-2xs text-faint">
                    {t("リセット:")} {resetTime(window.resets_at)}
                  </small>
                ) : null}
              </div>
            ))}
            {quota.data.credits ? (
              <p className="text-xs text-muted">
                {quota.data.credits.unlimited
                  ? t("クレジット: 無制限")
                  : quota.data.credits.has_credits
                    ? `${t("クレジット残高:")} ${quota.data.credits.balance || "—"}`
                    : t("クレジットなし")}
              </p>
            ) : null}
            {quota.data.limit_reached ? (
              <p className="text-xs text-danger">
                {t("使用枠に達しています。")}
                <br />
                {t(
                  "リセットを待つか、APIキー方式のProviderに切り替えてください。",
                )}
              </p>
            ) : null}
            <small className="text-2xs text-faint">
              {t("取得:")}{" "}
              {new Date(quota.data.captured_at * 1000).toLocaleTimeString()}
            </small>
          </>
        ) : null}
        {actionError ? (
          <p className="text-xs text-danger">{actionError}</p>
        ) : null}
        <p className="text-2xs text-faint">
          {t(
            "残量は OpenAI の非公開エンドポイントから取得しています。表示は保証されません。",
          )}
          <br />
          {t(
            "リセット時刻は端末のタイムゾーンで表示し、UTC も併記しています。",
          )}
        </p>
      </div>
    </Card>
  );
}

export function ChatGptLoginDialog({
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
  completeRef.current = onComplete;
  failedRef.current = onFailed;
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (login.status !== "pending") return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const delay = Math.max(2, login.interval || 2) * 1000;
    const poll = async () => {
      try {
        const status = await api.chatgptDevicePoll(login.login_id);
        if (cancelled) return;
        if (status.status === "complete") {
          completeRef.current(status.account);
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
        failedRef.current(errorMessage(error));
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
    <Card
      title={t("ChatGPT にログイン")}
      icon={<ExternalLink size={13} />}
      action={
        <Button variant="ghost" size="sm" onClick={onCancel}>
          {t("キャンセル")}
        </Button>
      }
    >
      <div className="flex flex-col gap-3 text-xs">
        <ChatGptWarning compact />
        <p className="text-muted">
          {t(
            "表示された URL を開き、ワンタイムコードを入力してください。この画面は自動で完了を確認します。",
          )}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <code className="rounded-md border border-dashed border-line bg-inset px-3 py-2 text-base tracking-widest text-fg">
            {login.user_code}
          </code>
          <Button variant="default" size="sm" onClick={() => void copyCode()}>
            {copied ? t("コピーしました") : t("コードをコピー")}
          </Button>
          <Button variant="default" size="sm" asChild>
            <a href={login.verification_url} target="_blank" rel="noreferrer">
              <ExternalLink size={12} />
              {t("認証ページを開く")}
            </a>
          </Button>
        </div>
        <p className="text-faint" aria-live="polite">
          {login.status === "pending"
            ? t("認証の完了を待っています…")
            : login.status}
        </p>
      </div>
    </Card>
  );
}
