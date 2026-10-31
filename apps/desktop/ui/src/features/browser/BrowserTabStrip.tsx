import { Globe, Plus, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import type { BrowserTabView } from "@/lib/types";

interface BrowserTabStripProps {
  tabs: BrowserTabView[];
  onSwitch: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
}

/** Chromium page tabs, driven by the backend's `tabs` / `new_tab` actions. */
export function BrowserTabStrip({
  tabs,
  onSwitch,
  onClose,
  onNew,
}: BrowserTabStripProps) {
  return (
    <div className="flex h-7 shrink-0 items-end gap-1 overflow-x-auto border-b border-line bg-inset/40 px-1.5">
      {tabs.map((tab) => {
        const label = tab.title?.trim() || tab.url || t("新しいタブ");
        return (
          <div
            key={tab.id}
            className={cn(
              "group flex h-6 min-w-24 max-w-52 shrink items-center gap-1 rounded-t-md border border-b-0 px-1.5 text-2xs transition-colors",
              tab.active
                ? "border-line bg-bg text-fg"
                : "border-transparent text-muted hover:bg-hover",
            )}
          >
            <button
              type="button"
              onClick={() => onSwitch(tab.id)}
              className="flex min-w-0 flex-1 items-center gap-1"
              title={tab.url ?? label}
            >
              <Globe size={10} className="shrink-0 text-faint" />
              <span className="truncate">{label}</span>
            </button>
            <button
              type="button"
              onClick={() => onClose(tab.id)}
              title={t("タブを閉じる")}
              aria-label={t("タブを閉じる")}
              className="shrink-0 rounded p-0.5 text-faint opacity-0 hover:text-fg group-hover:opacity-100 focus-visible:opacity-100"
            >
              <X size={10} />
            </button>
          </div>
        );
      })}
      <button
        type="button"
        onClick={onNew}
        title={t("新しいタブ")}
        aria-label={t("新しいタブ")}
        className="mb-0.5 grid size-5 shrink-0 place-items-center rounded text-faint hover:bg-hover hover:text-fg"
      >
        <Plus size={12} />
      </button>
    </div>
  );
}
