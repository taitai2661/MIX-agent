import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ChevronRight, ShieldCheck } from "lucide-react";
import { useState, type FormEvent } from "react";

import { ErrorBox, Field, Logo } from "@/components/shared";

export function Auth({
  setup,
  onDone,
}: {
  setup: boolean;
  onDone: (v: any) => void;
}) {
  const [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false);
  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    setBusy(true);
    setError(null);
    try {
      onDone(
        await api(setup ? "/setup/admin" : "/auth/login", "POST", {
          username: form.get("username"),
          password: form.get("password"),
        }),
      );
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
        <h1>{setup ? "ようこそ、MIX agentへ。" : "おかえりなさい。"}</h1>
        <p>
          {setup
            ? "まずは管理者アカウントを作成しましょう。"
            : "あなたのワークスペースにログインします。"}
        </p>
        <form onSubmit={submit}>
          <Field label="ユーザー名">
            <input
              name="username"
              required
              autoComplete="username"
              maxLength={100}
            />
          </Field>
          <Field
            label="パスワード"
            hint="12文字以上。API Keyとは別のパスワードです。"
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
            {busy ? "処理中…" : setup ? "ワークスペースを作成" : "ログイン"}
            <ChevronRight size={16} />
          </Button>
        </form>
        <small>
          <ShieldCheck size={13} /> 接続先のAI
          Providerへ送信した内容は、そのProviderの規約に従います。
        </small>
      </div>
    </main>
  );
}
