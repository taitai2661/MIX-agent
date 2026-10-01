import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ChevronRight, ShieldCheck } from "lucide-react";
import { useState, type FormEvent } from "react";

import { ErrorBox, Field, Logo } from "@/components/shared";
import { getLanguage, setLanguage, t, useLanguage } from "@/app/i18n";

export function Auth({
  setup,
  onDone,
}: {
  setup: boolean;
  onDone: (v: any) => void;
}) {
  const [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false);
  useLanguage();
  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    setBusy(true);
    setError(null);
    try {
      const result = await api(setup ? "/setup/admin" : "/auth/login", "POST", {
          username: form.get("username"),
          password: form.get("password"),
        });
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
        <label className="field"><span>{t("表示言語")}</span><select aria-label={t("表示言語")} value={getLanguage()} onChange={e => setLanguage(e.target.value as "ja" | "en")}><option value="ja">{t("日本語")}</option><option value="en">English</option></select></label>
        <h1>{t(setup ? "ようこそ、MIX agentへ。" : "おかえりなさい。")}</h1>
        <p>
          {t(setup
            ? "まずは管理者アカウントを作成しましょう。"
            : "あなたのワークスペースにログインします。")}
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
            <input
              name="password"
              type="password"
              required
              minLength={12}
              maxLength={256}
              autoComplete={setup ? "new-password" : "current-password"}
            />
          </Field>
          <ErrorBox error={error} />
          <Button disabled={busy}>
            {t(busy ? "処理中…" : setup ? "ワークスペースを作成" : "ログイン")}
            <ChevronRight size={16} />
          </Button>
        </form>
        <small>
          <ShieldCheck size={13} /> {t("接続先のAI Providerへ送信した内容は、そのProviderの規約に従います。")}
        </small>
      </div>
    </main>
  );
}
