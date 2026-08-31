import { api, setCSRF } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LogOut, ShieldCheck } from "lucide-react";
import { useState, type FormEvent } from "react";

type LoginEvent = { successful: boolean; created_at: string };

export function Account({
  user,
  onUserChange,
  onLogout,
}: {
  user: { username: string };
  onUserChange: (value: { username: string; csrf: string }) => void;
  onLogout: () => void;
}) {
  const history = useQuery<LoginEvent[]>({
    queryKey: ["/auth/login-history"],
    queryFn: () => api("/auth/login-history"),
  });
  const qc = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const [confirming, setConfirming] = useState<"others" | "all" | null>(null);
  const [busy, setBusy] = useState(false);

  async function submitPassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const next = String(form.get("new_password") || "");
    if (next !== String(form.get("confirm_password") || "")) {
      setError(new Error("新しいパスワードが一致しません"));
      return;
    }
    setBusy(true); setError(null); setNotice("");
    try {
      const result = await api<{ csrf?: string; relogin_required: boolean }>("/auth/password", "POST", {
        current_password: form.get("current_password"),
        new_password: next,
        revoke_all_sessions: form.get("revoke_all_sessions") === "on",
      });
      if (result.relogin_required) {
        onLogout();
        return;
      }
      if (result.csrf) setCSRF(result.csrf);
      event.currentTarget.reset();
      setNotice("パスワードを変更しました");
    } catch (value) { setError(value); } finally { setBusy(false); }
  }

  async function submitUsername(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    setBusy(true); setError(null); setNotice("");
    try {
      const result = await api<{ username: string; csrf: string }>("/auth/username", "POST", {
        current_password: form.get("current_password"), username: form.get("username"),
      });
      onUserChange(result);
      setNotice("ユーザー名を変更しました");
    } catch (value) { setError(value); } finally { setBusy(false); }
  }

  async function revoke(scope: "others" | "all") {
    setBusy(true); setError(null);
    try {
      const result = await api<{ relogin_required: boolean }>("/auth/sessions/revoke", "POST", { scope });
      if (result.relogin_required) onLogout();
      else setNotice("ほかのログインをすべて終了しました");
      setConfirming(null);
      qc.invalidateQueries({ queryKey: ["/auth/login-history"] });
    } catch (value) { setError(value); } finally { setBusy(false); }
  }

  return <>
    <Title title="アカウント・安全性" sub="ログイン情報と、アカウントの安全性を管理します。" />
    <ErrorBox error={error || history.error} />
    {notice && <p className="success">{notice}</p>}
    <form className="card" onSubmit={submitUsername}>
      <h3>ユーザー名を変更</h3><p>ログインに使用するユーザー名を変更します。</p>
      <Field label="新しいユーザー名"><input name="username" required maxLength={100} defaultValue={user.username} autoComplete="username" /></Field>
      <Field label="現在のパスワード"><input name="current_password" type="password" required minLength={12} maxLength={256} autoComplete="current-password" /></Field>
      <Button disabled={busy}>ユーザー名を保存</Button>
    </form>
    <form className="card" onSubmit={submitPassword}>
      <h3>パスワードを変更</h3><p>新しいパスワードは12文字以上で設定してください。</p>
      <Field label="現在のパスワード"><input name="current_password" type="password" required minLength={12} maxLength={256} autoComplete="current-password" /></Field>
      <Field label="新しいパスワード"><input name="new_password" type="password" required minLength={12} maxLength={256} autoComplete="new-password" /></Field>
      <Field label="新しいパスワード（確認）"><input name="confirm_password" type="password" required minLength={12} maxLength={256} autoComplete="new-password" /></Field>
      <label className="check"><input name="revoke_all_sessions" type="checkbox" /><span>変更後、すべての端末からログアウトする</span></label>
      <div className="form-actions"><Button disabled={busy}>パスワードを変更</Button></div>
    </form>
    <section className="card">
      <h3>ログイン中のセッション</h3><p>この端末以外、またはすべてのログインを終了できます。</p>
      {confirming ? <div className="notice"><ShieldCheck size={16} /><span>{confirming === "all" ? "この端末も含め、すべての端末で再ログインが必要になります。" : "この端末以外のログインを終了します。"}<span className="row-actions"><Button type="button" variant="destructive" disabled={busy} onClick={() => revoke(confirming)}>実行する</Button><Button type="button" variant="outline" disabled={busy} onClick={() => setConfirming(null)}>キャンセル</Button></span></span></div> : <div className="row-actions"><Button type="button" variant="outline" disabled={busy} onClick={() => setConfirming("others")}>他の端末をログアウト</Button><Button type="button" variant="destructive" disabled={busy} onClick={() => setConfirming("all")}>全端末をログアウト</Button></div>}
    </section>
    <section className="card">
      <h3>ログイン履歴</h3><p>直近100件のログイン結果です。接続元や端末情報は保存しません。</p>
      <div className="login-history">{history.data?.length ? history.data.map((event, index) => <div className="revision" key={`${event.created_at}-${index}`}><span>{event.successful ? "ログイン成功" : "ログイン失敗"}</span><time dateTime={event.created_at}>{new Date(event.created_at).toLocaleString("ja-JP")}</time></div>) : <p>まだログイン履歴はありません。</p>}</div>
    </section>
  </>;
}
