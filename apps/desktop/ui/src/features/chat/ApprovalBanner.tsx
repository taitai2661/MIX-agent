import { memo, useCallback, useState, type KeyboardEvent } from "react";
import {
  ChevronDown,
  ShieldAlert,
  Terminal,
  FileCode,
  Wrench,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { chord } from "@/lib/platform";
import { prefersReducedMotion } from "@/lib/motion";
import type { PendingApproval } from "@/lib/runEvents";
import { recordUxEvent } from "@/lib/uxEvents";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/layout";
import { ApprovalCountdown, useApprovalRemaining } from "./ApprovalCountdown";

export const ApprovalBanner = memo(function ApprovalBanner({
  approval,
}: {
  approval: PendingApproval;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remaining = useApprovalRemaining(approval.expiresAt);
  const expired = remaining === 0;

  const scrollToCard = () => {
    const card = document.querySelector<HTMLElement>('[role="alertdialog"]');
    if (card) {
      // Focus the card itself, not its first button. The first button is
      // 「拒否」, so the old behaviour armed a destructive answer and the very
      // next Enter/Space denied the request.
      card.scrollIntoView({
        behavior: prefersReducedMotion() ? "auto" : "smooth",
        block: "center",
      });
      card.focus();
    }
  };

  const decide = useCallback(
    async (grant: boolean, remember = false) => {
      const id = approval.approvalId;
      if (!id || busy || expired) return;
      setBusy(true);
      setError(null);
      try {
        await api.decideApproval({ id, grant, remember });
        recordUxEvent("approval_decided", {
          location: "banner",
          grant,
          remember,
        });
      } catch (err) {
        setError(errorMessage(err) || t("エラー"));
      } finally {
        setBusy(false);
      }
    },
    [approval.approvalId, busy, expired],
  );

  // Approval shortcuts are scoped to the banner: they only fire while focus is
  // inside it. A window-level listener used to approve a destructive tool when
  // the user pressed ⌘Enter to *send a message*, and Escape (used to close a
  // palette/dialog) could silently deny the request instead.
  //
  // The handler sits on the banner container, not on two of its four buttons:
  // the chips claimed Esc/⌘↵ while focus was on 「常に許可」 or 「詳細を確認」,
  // where neither did anything.
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      // A dangerous command needs the warning and the acknowledgement box in
      // the full card; the banner shortcut must not be a way around it.
      if (dangerous) {
        scrollToCard();
        return;
      }
      void decide(true);
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      void decide(false);
    }
  };

  const dangerous = approval.risk?.level === "dangerous";
  const isTerminal = approval.name.toLowerCase().includes("terminal");
  const isFileEdit =
    approval.name.toLowerCase().includes("edit") ||
    approval.name.toLowerCase().includes("file");

  return (
    // The shortcuts are scoped to focus inside this alert, deliberately: a
    // window-level listener approved tools when the user pressed ⌘Enter to
    // send a message. The container handles bubbled keys from its buttons.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <div
      role="alert"
      onKeyDown={onKeyDown}
      aria-keyshortcuts="Meta+Enter Control+Enter Escape"
      className="sticky top-0 z-20 flex shrink-0 items-center justify-between gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2 text-xs text-warning shadow-xs backdrop-blur-md"
    >
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <ShieldAlert
          size={15}
          className="shrink-0 text-warning animate-mix-pulse"
        />
        <span className="shrink-0 font-semibold">{t("承認待ち")}:</span>
        {dangerous && (
          <span className="shrink-0 rounded-sm bg-danger-soft px-1.5 py-0.5 text-2xs font-semibold text-danger">
            {t("危険")}
          </span>
        )}
        <span className="inline-flex min-w-0 items-center gap-1 rounded-md bg-surface px-1.5 py-0.5 font-mono text-2xs text-fg shadow-xs">
          {isTerminal ? (
            <Terminal size={11} className="shrink-0 text-warning" />
          ) : isFileEdit ? (
            <FileCode size={11} className="shrink-0 text-brand" />
          ) : (
            <Wrench size={11} className="shrink-0 text-muted" />
          )}
          <span className="truncate" title={approval.name}>
            {approval.name}
          </span>
        </span>
        {error && (
          <span className="min-w-0 truncate text-2xs text-danger" title={error}>
            ({error})
          </span>
        )}
      </div>
      <ApprovalCountdown expiresAt={approval.expiresAt} />
      <div className="flex shrink-0 items-center gap-1.5">
        <Button
          variant="ghost"
          size="xs"
          onClick={scrollToCard}
          className="gap-1 text-warning hover:bg-warning/20 hover:text-warning"
          title={t("詳細を確認")}
        >
          <span>{t("詳細を確認")}</span>
          <ChevronDown size={12} />
        </Button>
        <Button
          variant="default"
          size="xs"
          disabled={busy || !approval.approvalId || expired}
          onClick={() => void decide(false)}
          className="gap-1 border-warning/30 text-warning hover:bg-warning/20"
          title={`${t("拒否")} · Esc`}
        >
          <span>{t("拒否")}</span>
          <Kbd className="px-1 py-0 opacity-70">Esc</Kbd>
        </Button>
        {!dangerous && (
          <Button
            variant="ghost"
            size="xs"
            disabled={busy || !approval.approvalId || expired}
            onClick={() => void decide(true, true)}
            className="gap-1 text-warning hover:bg-warning/20"
            title={t("常に許可")}
          >
            <span>{t("常に許可")}</span>
          </Button>
        )}
        {dangerous ? (
          <Button
            variant="primary"
            size="xs"
            disabled={expired}
            onClick={scrollToCard}
            className="gap-1 shadow-xs"
            title={t("警告を確認してから許可してください")}
          >
            <span>{t("確認して許可")}</span>
            <ChevronDown size={12} />
          </Button>
        ) : (
          <Button
            variant="primary"
            size="xs"
            disabled={busy || !approval.approvalId || expired}
            onClick={() => void decide(true)}
            className="gap-1 shadow-xs"
            title={`${t("今回のみ許可")} · ${chord("Mod", "Enter")}`}
          >
            <span>{t("今回のみ許可")}</span>
            <Kbd className="px-1 py-0 border-on-solid/40 bg-transparent text-on-solid">
              {chord("Mod", "↵")}
            </Kbd>
          </Button>
        )}
      </div>
    </div>
  );
});
