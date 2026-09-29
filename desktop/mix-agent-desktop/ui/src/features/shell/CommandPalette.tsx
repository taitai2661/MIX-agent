import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Bot, CornerDownLeft, MessageSquare, Plus, Search, Settings2,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi, type SettingsTab, type Surface, type View } from "@/lib/ui";

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
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const openerRef = useRef<HTMLElement | null>(null);

  const conversations = useQuery({ queryKey: ["conversations"], queryFn: api.listConversations });

  useEffect(() => {
    if (open) {
      // Remember what had focus so closing hands it back instead of dropping
      // focus onto <body>.
      openerRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      setQuery("");
      setSelected(0);
      window.setTimeout(() => inputRef.current?.focus(), 10);
    } else {
      openerRef.current?.focus();
      openerRef.current = null;
    }
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
      label: id === "chat" ? "chat" : "desktop-agent",
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
      ...surfaces,
      ...views,
      ...settingsTabs,
      ...threads,
    ];
  }, [conversations.data, openSettings, setConversationId, setView, setSurface]);

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
      // A modal combobox with nowhere else to go: letting Tab escape would
      // leave focus behind the backdrop with no way back.
      e.preventDefault();
    }
  };

  const activeId = `palette-option-${selected}`;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/45 pt-[12vh] backdrop-blur-[1px]"
      onClick={() => setOpen(false)}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("コマンドパレット")}
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[60vh] w-full max-w-lg flex-col overflow-hidden rounded-xl border border-line bg-bg shadow-lg"
      >
        <input
          ref={inputRef}
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
          className="h-11 shrink-0 border-b border-line bg-transparent px-3 text-sm text-fg outline-none placeholder:text-faint"
        />
        <div
          id="palette-listbox"
          role="listbox"
          aria-label={t("コマンドパレット")}
          className="flex min-h-0 flex-col gap-0.5 overflow-y-auto p-1"
        >
          {filtered.map((item, i) => (
            <button
              type="button"
              role="option"
              id={`palette-option-${i}`}
              aria-selected={i === selected}
              key={item.id}
              onMouseEnter={() => setSelected(i)}
              onClick={() => {
                item.run();
                setOpen(false);
              }}
              className={cn(
                "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm",
                "text-fg transition-colors duration-75",
                i === selected ? "bg-hover" : "text-muted hover:bg-hover hover:text-fg",
              )}
            >
              <span className="shrink-0 text-faint">{item.icon}</span>
              <span className="truncate">{item.label}</span>
              {item.hint && <span className="ml-auto text-2xs text-faint">{item.hint}</span>}
              {i === selected && <CornerDownLeft size={12} className="shrink-0 text-faint" />}
            </button>
          ))}
          {filtered.length === 0 && (
            <p className="px-2 py-4 text-center text-xs text-faint">
              {t("一致する項目がありません")}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
