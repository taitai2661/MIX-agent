import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AlertTriangle } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { ErrorBox } from "@/components/shared";

interface QuitBlockedPayload {
  count?: unknown;
}

/**
 * Confirmation for quitting with live Runs.
 *
 * Rust refuses the first `ExitRequested` (menu Quit, Cmd+Q, tray Quit) and
 * emits `app://quit-blocked` instead. Without this dialog the user saw the app
 * simply ignore the command; with it they can either cancel or explicitly
 * stop the Runs and quit.
 */
export function QuitGuard() {
  const [open, setOpen] = useState(false);
  const [count, setCount] = useState(0);
  const [quitting, setQuitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<QuitBlockedPayload>("app://quit-blocked", (event) => {
      if (!active) return;
      const raw = event.payload?.count;
      const next = typeof raw === "number" && Number.isFinite(raw) ? raw : 1;
      setCount(Math.max(1, Math.floor(next)));
      setError(null);
      setOpen(true);
    }).then((un) => {
      if (active) unlisten = un;
      else un();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  const quit = async () => {
    setQuitting(true);
    setError(null);
    try {
      await api.quitApp();
      // The process exits; if the call returns because the platform vetoed
      // it, keep the dialog up with the reason instead of pretending success.
    } catch (err) {
      setError(errorMessage(err));
      setQuitting(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !next && !quitting && setOpen(false)}
    >
      <DialogContent className="max-w-md">
        <div className="mb-3 flex items-center gap-2">
          <span className="grid size-8 place-items-center rounded-lg bg-warning-soft text-warning">
            <AlertTriangle size={15} aria-hidden="true" />
          </span>
          <DialogTitle>{t("実行中のタスクがあります")}</DialogTitle>
        </div>
        <DialogDescription>
          {tf(
            "{count} 件の実行が進行中です。終了すると実行は中断され、次回起動時にチェックポイントから再開できます。",
            { count },
          )}
        </DialogDescription>
        {error && <ErrorBox error={error} className="mt-3" />}
        <DialogFooter>
          <Button
            variant="default"
            size="sm"
            disabled={quitting}
            onClick={() => setOpen(false)}
          >
            {t("キャンセル")}
          </Button>
          <Button
            variant="danger"
            size="sm"
            disabled={quitting}
            onClick={() => void quit()}
          >
            {quitting ? t("終了しています…") : t("実行を停止して終了")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
