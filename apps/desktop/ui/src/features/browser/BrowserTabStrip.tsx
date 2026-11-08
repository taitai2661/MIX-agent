import { Globe, Plus, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import type { BrowserTabView } from "@/lib/types";
import { Spinner } from "@/components/ui/spinner";

interface BrowserTabStripProps {
  tabs: BrowserTabView[];
  /** A navigation/tab action is in flight; show a spinner on the active tab. */
  loading?: boolean;
  onSwitch: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
}

const AVATAR_TONES = [
  "bg-sky-500",
  "bg-violet-500",
  "bg-emerald-500",
  "bg-amber-500",
  "bg-rose-500",
  "bg-cyan-600",
];

/** One deterministic letter + colour per host; no network favicon fetch.
 *
 *  Fetching `/favicon.ico` from the WebView would bypass the browser's egress
 *  gate, so a page could make the app dial a private address via
 *  `<link rel="icon">`. The host text is enough to tell tabs apart.
 */
export function tabAvatar(url: string | null): {
  label: string;
  tone: string;
} {
  if (!url || url === "about:blank") return { label: "", tone: "" };
  let host = url;
  try {
    host = new URL(url).hostname.replace(/^www\./, "");
  } catch {
    // Keep the raw string; a malformed URL still gets a stable avatar.
  }
  const label = (host[0] ?? "?").toUpperCase();
  let hash = 0;
  for (let i = 0; i < host.length; i += 1) {
    hash = (hash * 31 + host.charCodeAt(i)) >>> 0;
  }
  return { label, tone: AVATAR_TONES[hash % AVATAR_TONES.length] };
}

/** Native WebView tabs, driven by the backend's `tabs` / `new_tab` actions.
 *
 *  Page tabs are a second row under the unified panel tab bar. The panel owns
 *  the close/full controls, so this strip only handles the pages themselves.
 */
export function BrowserTabStrip({
  tabs,
  loading = false,
  onSwitch,
  onClose,
  onNew,
}: BrowserTabStripProps) {
  return (
    <div className="flex h-8 shrink-0 items-center gap-1 border-b border-line bg-inset/40 px-1.5">
      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {tabs.map((tab) => {
          const label = tab.title?.trim() || tab.url || t("新しいタブ");
          const avatar = tabAvatar(tab.url);
          return (
            <div
              key={tab.id}
              className={cn(
                "group flex h-6 min-w-24 max-w-52 shrink items-center gap-1.5 rounded-md border px-2 text-2xs transition-colors",
                tab.active
                  ? "border-line bg-surface text-fg shadow-xs"
                  : "border-transparent text-muted hover:bg-hover hover:text-fg",
              )}
            >
              {tab.active && loading ? (
                <Spinner size={11} />
              ) : avatar.label ? (
                <span
                  aria-hidden="true"
                  className={cn(
                    "grid size-3.5 shrink-0 place-items-center rounded-[3px] text-[8px] font-semibold text-white",
                    avatar.tone,
                  )}
                >
                  {avatar.label}
                </span>
              ) : (
                <Globe size={11} className="shrink-0 text-faint" />
              )}
              <button
                type="button"
                onClick={() => onSwitch(tab.id)}
                onAuxClick={(event) => {
                  // Middle-click closes, as in Chrome.
                  if (event.button === 1) {
                    event.preventDefault();
                    onClose(tab.id);
                  }
                }}
                className="flex min-w-0 flex-1 items-center text-start"
                title={tab.url ?? label}
              >
                <span className="truncate">{label}</span>
              </button>
              <button
                type="button"
                onClick={() => onClose(tab.id)}
                title={t("タブを閉じる")}
                aria-label={t("タブを閉じる")}
                className={cn(
                  "shrink-0 rounded p-0.5 text-faint hover:bg-active hover:text-fg",
                  tab.active
                    ? "opacity-100"
                    : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
                )}
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
          className="grid size-6 shrink-0 place-items-center rounded-md text-faint hover:bg-hover hover:text-fg"
        >
          <Plus size={13} />
        </button>
      </div>
    </div>
  );
}
