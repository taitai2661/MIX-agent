import { useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Archive, Download, RefreshCw, Upload } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { Card, Row, Section } from "@/components/layout";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export function BackupSettings() {
  const [password, setPassword] = useState("");
  const [path, setPath] = useState("");
  const [restorePath, setRestorePath] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async (kind: "create" | "restore") => {
    setError(null);
    setResult(null);
    setBusy(true);
    try {
      if (kind === "create") {
        const picked = await saveDialog({
          defaultPath: "mix-agent.mixbak",
          filters: [{ name: "Backup", extensions: ["mixbak"] }],
        });
        if (!picked) return;
        const out = await api.createBackup({ destination: picked, password });
        setResult(`Saved ${out.bytes.toLocaleString()} bytes (${out.tables} tables) to ${out.path}`);
      } else {
        const out = await api.restoreBackup({ source: restorePath, password });
        setResult(
          `Verified ${out.tables} tables from ${new Date(out.created_at).toLocaleString()}. ` +
            `The app restarts to load the restored data.`,
        );
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section>
      <Card title="Backup / Restore" icon={<Archive size={13} />}>
        <p className="m-0 text-2xs text-faint">
          The whole database is snapshotted, compressed and encrypted (Argon2id + AES-256-GCM).
          API keys stay in the OS Keychain and are not included.
        </p>
        <Input
          type="password"
          placeholder="Backup password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <Row className="gap-1.5">
          <Input
            className="min-w-0 flex-1"
            placeholder="/path/to/backup.mixbak"
            value={path}
            onChange={(e) => setPath(e.target.value)}
          />
          <Button
            variant="primary"
            disabled={busy || !password}
            onClick={() => void run("create")}
            title="Create an encrypted backup"
          >
            <Download size={12} /> {t("作成")}
          </Button>
        </Row>
        <Row className="gap-1.5">
          <Input
            className="min-w-0 flex-1"
            placeholder="Existing .mixbak to restore"
            value={restorePath}
            onChange={(e) => setRestorePath(e.target.value)}
          />
          <Button
            disabled={busy || !restorePath || !password}
            onClick={() => void run("restore")}
            title="Verify a backup and reload it on restart"
          >
            <Upload size={12} /> Restore
          </Button>
        </Row>
        {result && <Alert tone="success">{result}</Alert>}
        {error && <Alert tone="danger">{error}</Alert>}
        {result?.includes("restarts") && (
          <Row>
            <Button disabled={busy} onClick={() => void api.relaunchApp()}>
              <RefreshCw size={12} /> Restart now
            </Button>
          </Row>
        )}
      </Card>
    </Section>
  );
}
