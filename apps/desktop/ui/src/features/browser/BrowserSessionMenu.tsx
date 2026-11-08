import {
  MoreVertical,
  Pin,
  PinOff,
  Plus,
  RefreshCw,
  Settings2,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { WebviewBrowser } from "./useWebviewBrowser";

interface BrowserSessionMenuProps {
  browser: WebviewBrowser;
  workspaceId: string | null;
  autoOpen: boolean;
  onToggleAutoOpen: () => void;
  onResetRatio: () => void;
  onClearData: () => void;
  onOpenSettings: () => void;
  onOpenChange: (open: boolean) => void;
}

/** The browser's overflow menu. Keeps the toolbar to Chrome's essentials. */
export function BrowserSessionMenu({
  browser,
  workspaceId,
  autoOpen,
  onToggleAutoOpen,
  onResetRatio,
  onClearData,
  onOpenSettings,
  onOpenChange,
}: BrowserSessionMenuProps) {
  return (
    <DropdownMenu onOpenChange={onOpenChange}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          title={t("ブラウザメニュー")}
          aria-label={t("ブラウザメニュー")}
        >
          <MoreVertical size={14} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-52">
        <DropdownMenuItem onSelect={() => void browser.newTab()}>
          <Plus size={13} />
          {t("新しいタブ")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void browser.reload()}>
          <RefreshCw size={13} />
          {t("再読み込み")}
        </DropdownMenuItem>
        {browser.navigating && (
          <DropdownMenuItem onSelect={() => void browser.stop()}>
            <Square size={13} />
            {t("停止")}
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onResetRatio}>
          {t("50%に戻す")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>
          {t("ブラウザセッション")}
          {workspaceId ? (
            <span className="ms-1 font-mono text-2xs text-faint">
              {workspaceId.slice(0, 8)}
            </span>
          ) : null}
        </DropdownMenuLabel>
        <DropdownMenuItem onSelect={onClearData}>
          <Trash2 size={13} className="text-danger" />
          <span className="text-danger">{t("閲覧データを削除")}</span>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void browser.closeSession()}>
          <X size={13} />
          {t("全タブを閉じる")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onToggleAutoOpen}>
          {autoOpen ? <Pin size={13} /> : <PinOff size={13} />}
          {autoOpen
            ? t("エージェント使用時の自動表示をオフ")
            : t("エージェント使用時に自動表示")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onOpenSettings}>
          <Settings2 size={13} />
          {t("ブラウザ設定を開く")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
