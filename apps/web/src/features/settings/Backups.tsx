import { binary } from "@/app/api";
import { Button } from "@/components/button";
import { Download } from "lucide-react";
import { useState } from "react";

import { ErrorBox, Field, Title } from "@/components/shared";

export function Backups() {
  const [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [notice, setNotice] = useState("");
  return (
    <>
      <Title
        title="バックアップ"
        sub="会話・設定・ファイルを、暗号化して保管します。"
      />
      <div className="notice">
        復号パスフレーズをなくすと復元できません。バックアップにはAPI
        Keyの復旧に必要な鍵も含まれます。安全な場所で管理してください。
      </div>
      <ErrorBox error={error} />
      {notice && <p className="success">{notice}</p>}
      <form
        className="card"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          const f = new FormData(e.currentTarget);
          try {
            const r = await binary("/backups", {
              passphrase: f.get("passphrase"),
            });
            const url = URL.createObjectURL(await r.blob());
            const a = document.createElement("a");
            a.href = url;
            a.download = "mix-agent-backup.mix";
            a.click();
            setTimeout(() => URL.revokeObjectURL(url), 1000);
            setNotice("暗号化バックアップを作成しました");
          } catch (e) {
            setError(e);
          } finally {
            setBusy(false);
          }
        }}
      >
        <h3>
          <Download size={18} />
          バックアップを作成
        </h3>
        <Field label="暗号化パスフレーズ">
          <input name="passphrase" type="password" minLength={12} required />
        </Field>
        <Button disabled={busy}>ダウンロード</Button>
      </form>
      <form
        className="card"
        onSubmit={async (e) => {
          e.preventDefault();
          if (
            !confirm(
              "現在のデータをバックアップ内容に置き換えます。現在のバックアップを確保しましたか？",
            )
          )
            return;
          setBusy(true);
          setError(null);
          try {
            await binary("/backups/restore", new FormData(e.currentTarget));
            location.reload();
          } catch (e) {
            setError(e);
          } finally {
            setBusy(false);
          }
        }}
      >
        <h3>バックアップから復元</h3>
        <Field label="バックアップファイル">
          <input name="file" type="file" accept=".mix" required />
        </Field>
        <Field label="復号パスフレーズ">
          <input name="passphrase" type="password" minLength={12} required />
        </Field>
        <Button variant="outline" disabled={busy}>
          検証して復元
        </Button>
      </form>
    </>
  );
}
