import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ChevronRight, Eye, EyeOff, ShieldCheck } from "lucide-react";
import { useState, type FormEvent } from "react";

import { ErrorBox, Field, Logo } from "@/components/shared";
import { getLanguage, setLanguage, t, useLanguage } from "@/app/i18n";

export function Auth({
  setup,
  sessionExpired = false,
  onDone,
}: {
  setup: boolean;
  sessionExpired?: boolean;
  onDone: (v: { username: string; csrf: string }) => void;
}) {
  const [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [password, setPassword] = useState(""),
    [confirmPassword, setConfirmPassword] = useState(""),
    [showPassword, setShowPassword] = useState(false);
  useLanguage();
  const passwordMismatch =
    setup && confirmPassword.length > 0 && confirmPassword !== password;
  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (setup && password !== confirmPassword) {
      setError(new Error(t("パスワードが一致しません")));
      return;
    }
    const form = new FormData(e.currentTarget);
    setBusy(true);
    setError(null);
    try {
      const result = await api<{ username: string; csrf: string }>(
        setup ? "/setup/admin" : "/auth/login",
        "POST",
        {
          username: form.get("username"),
          password: setup ? password : form.get("password"),
        },
      );
      onDone(result);
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="auth">
      <div className="auth-card">
        <Logo />
        <p className="eyebrow">YOUR AI. YOUR SPACE.</p>
        <label className="field">
          <span>{t("表示言語")}</span>
          <select
            aria-label={t("表示言語")}
            value={getLanguage()}
            onChange={(e) => setLanguage(e.target.value as "ja" | "en")}
          >
            <option value="ja">{t("日本語")}</option>
            <option value="en">English</option>
          </select>
        </label>
        <h1>
          {t(
            setup
              ? "ようこそ、MIX agentへ。"
              : sessionExpired
                ? "セッションの有効期限が切れました。"
                : "おかえりなさい。",
          )}
        </h1>
        <p>
          {t(
            setup
              ? "まずは管理者アカウントを作成しましょう。"
              : sessionExpired
                ? "入力中の内容は保存されています。再ログインすると元の画面に戻ります。"
                : "あなたのワークスペースにログインします。",
          )}
        </p>
        <form onSubmit={submit}>
          <Field label={t("ユーザー名")}>
            <input
              name="username"
              required
              autoComplete="username"
              maxLength={100}
            />
          </Field>
          <Field
            label={t("パスワード")}
            hint={t("12文字以上。API Keyとは別のパスワードです。")}
          >
            <div className={setup ? "password-input" : undefined}>
              <input
                name="password"
                type={setup && showPassword ? "text" : "password"}
                required
                minLength={12}
                maxLength={256}
                autoComplete={setup ? "new-password" : "current-password"}
                value={setup ? password : undefined}
                onChange={
                  setup
                    ? (event) => {
                        setPassword(event.target.value);
                        setError(null);
                      }
                    : undefined
                }
              />
              {setup && (
                <button
                  type="button"
                  className="password-toggle"
                  aria-label={t(
                    showPassword ? "パスワードを隠す" : "パスワードを表示",
                  )}
                  aria-pressed={showPassword}
                  onClick={() => setShowPassword((value) => !value)}
                >
                  {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
                </button>
              )}
            </div>
          </Field>
          {setup && (
            <Field label={t("パスワード（確認）")}>
              <input
                name="confirm_password"
                type={showPassword ? "text" : "password"}
                required
                minLength={12}
                maxLength={256}
                autoComplete="new-password"
                value={confirmPassword}
                aria-invalid={passwordMismatch}
                onChange={(event) => {
                  setConfirmPassword(event.target.value);
                  setError(null);
                }}
              />
              {passwordMismatch && (
                <small className="field-error">
                  {t("パスワードが一致しません")}
                </small>
              )}
            </Field>
          )}
          <ErrorBox error={error} />
          <Button disabled={busy || passwordMismatch}>
            {t(busy ? "処理中…" : setup ? "ワークスペースを作成" : "ログイン")}
            <ChevronRight size={16} />
          </Button>
        </form>
        <small>
          <ShieldCheck size={13} />{" "}
          {t(
            "接続先のAI Providerへ送信した内容は、そのProviderの規約に従います。",
          )}
        </small>
      </div>
    </main>
  );
}
