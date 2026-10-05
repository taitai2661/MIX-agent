import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Bot, CornerDownLeft, Keyboard, MessageSquare, Plus, Search, Settings2,
} from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { CONVERSATIONS_QUERY_KEY } from "@/lib/queries";
import { useUi, type SettingsTab, type Surface, type View } from "@/lib/ui";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { ListRow, Kbd } from "@/components/layout";

interface PaletteItem {
  id: string;
  label: string;
  hint?: string;
  icon: React.ReactNode;
  run: () => void;
}

const VIEW_LABELS: Partial<Record<View, string>> = {
  chat: "チャット",
  agents: "エージェント",
  projects: "プロジェクト",
  schedules: "スケジュール",
  settings: "設定",
};

/** Jump commands that open the Settings pane on a specific tab. */
const SETTINGS_TARGETS: { tab: SettingsTab; label: string }[] = [
  { tab: "providers", label: "プロバイダー" },
  { tab: "models", label: "モデル" },
  { tab: "tools", label: "ツール" },
  { tab: "approvals", label: "承認" },
  { tab: "mcp", label: "MCP" },
  { tab: "memory", label: "メモリ" },
  { tab: "knowledge", label: "ナレッジ" },
  { tab: "skills", label: "スキル" },
  { tab: "browser", label: "ブラウザ" },
  { tab: "diagnostics", label: "診断" },
];

export function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  const setOpen = useUi((s) => s.setPaletteOpen);
  const setView = useUi((s) => s.setView);
  const setConversationId = useUi((s) => s.setConversationId);
  const openSettings = useUi((s) => s.openSettings);
  const setSurface = useUi((s) => s.setSurface);
  const setShortcutsOpen = useUi((s) => s.setShortcutsOpen);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);

  // Only fetched while the palette is open: it is mounted permanently, and as
  // a fifth observer of this key it made a `list_conversations` call on every
  // app start for a list the user had not asked to see.
  const conversations = useQuery({
    queryKey: CONVERSATIONS_QUERY_KEY,
    queryFn: api.listConversations,
    enabled: open,
  });

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelected(0);
  }, [open]);

  const items = useMemo<PaletteItem[]>(() => {
    const views: PaletteItem[] = (Object.keys(VIEW_LABELS) as View[]).map((id) => ({
      id: `view:${id}`,
      label: t(VIEW_LABELS[id]!),
      icon: <Search size={14} />,
      run: () => setView(id),
    }));
    const settingsTabs: PaletteItem[] = SETTINGS_TARGETS.map(({ tab, label }) => ({
      id: `settings:${tab}`,
      label: t(label),
      hint: t("設定"),
      icon: <Settings2 size={14} />,
      run: () => openSettings(tab),
    }));
    const threads: PaletteItem[] = (conversations.data ?? []).slice(0, 30).map((c) => ({
      id: `thread:${c.id}`,
      label: c.title,
      hint: t("チャット"),
      icon: <MessageSquare size={14} />,
      run: () => {
        setConversationId(c.id);
        setView("chat");
      },
    }));
    const surfaces: PaletteItem[] = (["chat", "desktop-agent"] as Surface[]).map((id) => ({
      id: `surface:${id}`,
      label: id === "chat" ? "チャット" : "エージェント",
      hint: t("サーフェスを切り替え"),
      icon: id === "chat" ? <MessageSquare size={14} /> : <Bot size={14} />,
      run: () => setSurface(id),
    }));
    return [
      {
        id: "action:new-thread",
        label: t("新規スレッド"),
        hint: "⌘N",
        icon: <Plus size={14} />,
        run: () => {
          setView("chat");
          window.dispatchEvent(new CustomEvent("mix:new-thread"));
        },
      },
      {
        id: "action:shortcuts",
        label: t("ショートカット一覧"),
        hint: "?",
        icon: <Keyboard size={14} />,
        run: () => setShortcutsOpen(true),
      },
      ...surfaces,
      ...views,
      ...settingsTabs,
      ...threads,
    ];
  }, [conversations.data, openSettings, setConversationId, setView, setSurface, setShortcutsOpen]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items.slice(0, 12);
    return items.filter((item) => item.label.toLowerCase().includes(q)).slice(0, 20);
  }, [items, query]);

  useEffect(() => {
    setSelected(0);
  }, [query]);

  if (!open) return null;

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((s) => Math.min(s + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[selected];
      if (item) {
        item.run();
        setOpen(false);
      }
    } else if (e.key === "Escape") {
      setOpen(false);
    } else if (e.key === "Tab") {
      // A modal combobox with nowhere else to go. Radix's focus trap keeps Tab
      // inside the dialog, so swallowing it here is redundant, but a combobox
      // that moved focus on Tab would still be wrong.
      e.preventDefault();
    }
  };

  const activeId = `palette-option-${selected}`;

  return (
    // A real dialog rather than a hand-rolled `fixed inset-0` overlay: the old
    // one had no focus trap, no portal, and no `inert` background, which is why
    // it had to swallow Tab itself and hand focus back by hand.
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        aria-label={t("コマンドパレット")}
        className="top-[12vh] flex max-h-[60vh] w-full max-w-xl -translate-y-0 flex-col overflow-hidden p-0"
      >
        <DialogTitle className="sr-only">{t("コマンドパレット")}</DialogTitle>
        <div className="flex shrink-0 items-center gap-2.5 border-b border-line bg-inset px-3.5">
          <Search size={15} className="shrink-0 text-faint" aria-hidden="true" />
          <input
            autoFocus
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-listbox"
            aria-activedescendant={filtered.length > 0 ? activeId : undefined}
            aria-autocomplete="list"
            aria-label={t("コマンドを検索…")}
            placeholder={t("コマンドを検索…")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            className="h-11 min-w-0 flex-1 bg-transparent text-sm text-fg outline-none placeholder:text-faint"
          />
          <Kbd className="hidden sm:inline-flex">Esc</Kbd>
        </div>
        <div
          id="palette-listbox"
          role="listbox"
          aria-label={t("コマンドパレット")}
          className="flex min-h-0 flex-col gap-0.5 overflow-y-auto p-1.5"
        >
          {filtered.map((item, i) => (
            <ListRow
              role="option"
              id={`palette-option-${i}`}
              aria-selected={i === selected}
              key={item.id}
              selected={i === selected}
              onMouseEnter={() => setSelected(i)}
              onClick={() => {
                item.run();
                setOpen(false);
              }}
            >
              <span className="shrink-0 text-faint">{item.icon}</span>
              <span className="truncate">{item.label}</span>
              {item.hint && <span className="ms-auto text-xs text-faint">{item.hint}</span>}
              {i === selected && <CornerDownLeft size={12} className="shrink-0 text-faint" />}
            </ListRow>
          ))}
          {filtered.length === 0 && (
            <p className="px-2 py-4 text-center text-xs text-faint">
              {t("一致する項目がありません")}
            </p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
