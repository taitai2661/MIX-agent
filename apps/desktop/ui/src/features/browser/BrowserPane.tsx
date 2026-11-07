import { useState } from "react";
import { Globe, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { Card, Section } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";

/**
 * Browser settings for the native child WebView.
 *
 * The old installer/profile/live-view cards are gone: the page is rendered by
 * the platform WebView and there is no Chromium download to manage.
 */
export function BrowserPane() {
  const workspaceId = useUi((s) => s.workspaceId);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { confirm, dialog } = useConfirm();

  const clearData = async () => {
    if (!workspaceId) return;
    const ok = await confirm({
      title: t("このブラウザの閲覧データを削除しますか？"),
      description: t(
        "Cookie・localStorage・キャッシュが消えます。開いているタブも閉じます。",
      ),
      confirmLabel: t("削除"),
      tone: "danger",
    });
    if (!ok) return;
    setBusy(true);
    setError(null);
    try {
      await api.browserClearData(workspaceId);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section>
      <Card title={t("内蔵ブラウザ")} icon={<Globe size={13} />}>
        <div className="flex flex-col gap-3">
          <p className="m-0 text-xs text-muted">
            {t(
              "ページはアプリ内のネイティブ WebView で表示されます。エージェントと同じ画面を共有し、スクロール・入力・IME はそのまま動きます。",
            )}
          </p>
          <p className="m-0 text-xs text-muted">
            {t(
              "Cookie とログインはこのアプリ専用のブラウザデータに保存されます。通常の Chrome とは共有されません。",
            )}
          </p>
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              disabled={!workspaceId || busy}
              onClick={() => void clearData()}
            >
              {busy ? <Spinner size={12} /> : <Trash2 size={12} />}
              {t("閲覧データを削除")}
            </Button>
            {!workspaceId && (
              <span className="text-2xs text-faint">
                {t("スレッドを開始すると使えます。")}
              </span>
            )}
          </div>
          {error && <p className="m-0 text-xs text-danger">{error}</p>}
        </div>
      </Card>
      {dialog}
    </Section>
  );
}
