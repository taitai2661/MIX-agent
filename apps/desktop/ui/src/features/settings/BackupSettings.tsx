import { useState } from "react";
import { Archive, Download, FolderOpen, RefreshCw, Upload } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { Card, Row, Section } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useConfirm } from "@/components/ui/dialog";

export function BackupSettings() {
  const [password, setPassword] = useState("");
  const [path, setPath] = useState("");
  const [restorePath, setRestorePath] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [needsRestart, setNeedsRestart] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const { confirm, dialog } = useConfirm();

  const requestRestore = async () => {
    const ok = await confirm({
      title: t("バックアップを復元しますか？"),
      description: t(
        "選択したバックアップを検証して復元の準備をします。再起動すると現在の会話・設定・記憶は復元時点の内容に置き換わります。復元後に作成したデータは元に戻せません。",
      ),
      confirmLabel: t("復元する"),
      tone: "danger",
    });
    if (ok) void run("restore");
  };

  const requestRelaunch = async () => {
    const ok = await confirm({
      title: t("復元を適用して再起動しますか？"),
      description: t(
        "現在のデータはバックアップの内容に置き換わります。この操作は元に戻せません。",
      ),
      confirmLabel: t("再起動して適用"),
      tone: "danger",
    });
    if (ok) void api.relaunchApp();
  };

  const run = async (kind: "create" | "restore") => {
    setError(null);
    setResult(null);
    setNeedsRestart(false);
    setBusy(true);
    try {
      if (kind === "create") {
        // The typed destination used to be ignored outright: the dialog always
        // opened at the default name, so whatever the user entered in the path
        // field silently had no effect. Honour it when present.
        const picked = path.trim()
          ? path.trim()
          : await api.pickSaveFile({
              defaultName: "mix-agent.mixbak",
              filters: [{ name: "Backup", extensions: ["mixbak"] }],
            });
        if (!picked) return;
        const out = await api.createBackup({ destination: picked, password });
        setResult(
          tf("{bytes} バイト（{tables} テーブル）を {path} に保存しました", {
            bytes: out.bytes.toLocaleString(),
            tables: out.tables,
            path: out.path,
          }),
        );
      } else {
        const out = await api.restoreBackup({ source: restorePath, password });
        setResult(
          tf(
            "{tables} テーブルを検証しました（{date}）。復元データを読み込むためにアプリを再起動します。",
            {
              tables: out.tables,
              date: new Date(out.created_at).toLocaleString(),
            },
          ),
        );
        // Decided by the operation, not by the wording of the message — a
        // translated string must not be able to hide the restart button.
        setNeedsRestart(true);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section>
      <Card title={t("バックアップ / 復元")} icon={<Archive size={13} />}>
        <p className="m-0 text-xs text-muted">
          {t(
            "データベース全体をスナップショットし、圧縮・暗号化します (Argon2id + AES-256-GCM)。API キーは OS キーチェーンに残るため含まれません。",
          )}
        </p>
        <Input
          type="password"
          aria-label={t("バックアップパスワード")}
          placeholder={t("バックアップパスワード")}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <Row className="gap-1.5">
          <Input
            className="min-w-0 flex-1"
            aria-label={t("保存先パス")}
            placeholder="/path/to/backup.mixbak"
            value={path}
            onChange={(e) => setPath(e.target.value)}
          />
          <Button
            variant="primary"
            disabled={busy || !password}
            onClick={() => void run("create")}
            title={t("暗号化バックアップを作成")}
          >
            <Download size={12} /> {t("作成")}
          </Button>
        </Row>
        <Row className="gap-1.5">
          <Input
            className="min-w-0 flex-1"
            aria-label={t("復元する既存の .mixbak")}
            placeholder={t("復元する既存の .mixbak")}
            value={restorePath}
            onChange={(e) => setRestorePath(e.target.value)}
          />
          <Button
            disabled={busy}
            title={t("ファイルを選択")}
            onClick={async () => {
              const picked = await api.pickOpenFile({
                filters: [{ name: "Backup", extensions: ["mixbak"] }],
              });
              if (picked.length > 0) setRestorePath(picked[0]);
            }}
          >
            <FolderOpen size={12} /> {t("参照")}
          </Button>
          <Button
            disabled={busy || !restorePath || !password}
            onClick={() => void requestRestore()}
            title={t("バックアップを検証し、再起動時に復元")}
          >
            <Upload size={12} /> {t("復元")}
          </Button>
        </Row>
        {result && <Alert tone="success">{result}</Alert>}
        {error && <ErrorBox error={error} />}
        {needsRestart && (
          <Row>
            <Button disabled={busy} onClick={() => void requestRelaunch()}>
              <RefreshCw size={12} /> {t("今すぐ再起動")}
            </Button>
          </Row>
        )}
      </Card>
      {dialog}
    </Section>
  );
}
