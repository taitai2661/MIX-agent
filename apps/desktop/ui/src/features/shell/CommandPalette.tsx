import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Bot,
  CornerDownLeft,
  FolderTree,
  Keyboard,
  MessageSquare,
  PanelLeft,
  Plus,
  Search,
  Settings2,
  Sparkles,
} from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { CONVERSATIONS_QUERY_KEY, MODELS_QUERY_KEY } from "@/lib/queries";
import { useUi, type SettingsTab, type Surface, type View } from "@/lib/ui";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { ListRow, Kbd } from "@/components/layout";
import { ListboxSkeleton } from "@/components/common";
import { EmptyState } from "@/components/shared";

interface PaletteItem {
  id: string;
  label: string;
  hint?: string;
  /** A keyboard shortcut, rendered as a `Kbd` chip rather than plain text. */
  keys?: string;
  icon: React.ReactNode;
  kind: "action" | "surface" | "view" | "settings" | "model" | "thread";
  run: () => void;
}

const KIND_LABEL: Record<PaletteItem["kind"], string> = {
  action: "アクション",
  surface: "サーフェス",
  view: "表示",
  settings: "設定",
  model: "モデル",
  thread: "スレッド",
};

/** Subsequence fuzzy match: `mdl` matches "model". Case-insensitive. */
function fuzzyMatch(haystack: string, needle: string): boolean {
  if (!needle) return true;
  let j = 0;
  for (let i = 0; i < haystack.length && j < needle.length; i += 1) {
    if (haystack[i] === needle[j]) j += 1;
  }
  return j === needle.length;
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
  const setModelId = useUi((s) => s.setModelId);
  const openSettings = useUi((s) => s.openSettings);
  const setSurface = useUi((s) => s.setSurface);
  const setShortcutsOpen = useUi((s) => s.setShortcutsOpen);
  const toggleSidebar = useUi((s) => s.toggleSidebar);
  const toggleFileTree = useUi((s) => s.toggleFileTree);
  const locale = useUi((s) => s.locale);
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
  const models = useQuery({
    queryKey: MODELS_QUERY_KEY,
    queryFn: () => api.listModels(),
    enabled: open,
  });

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelected(0);
  }, [open]);

  const items = useMemo<PaletteItem[]>(() => {
    // `t()` reads the display language from module state, so mention `locale`
    // to recompute the labels on a language switch.
    void locale;
    const views: PaletteItem[] = (Object.keys(VIEW_LABELS) as View[]).map(
      (id) => ({
        id: `view:${id}`,
        label: t(VIEW_LABELS[id]!),
        icon: <Search size={15} />,
        kind: "view",
        run: () => setView(id),
      }),
    );
    const settingsTabs: PaletteItem[] = SETTINGS_TARGETS.map(
      ({ tab, label }) => ({
        id: `settings:${tab}`,
        label: t(label),
        hint: t("設定"),
        icon: <Settings2 size={15} />,
        kind: "settings",
        run: () => openSettings(tab),
      }),
    );
    const threads: PaletteItem[] = (conversations.data ?? [])
      .slice(0, 30)
      .map((c) => ({
        id: `thread:${c.id}`,
        label: c.title,
        hint: t("チャット"),
        icon: <MessageSquare size={15} />,
        kind: "thread",
        run: () => {
          setConversationId(c.id);
          setView("chat");
        },
      }));
    const surfaces: PaletteItem[] = (
      ["chat", "desktop-agent"] as Surface[]
    ).map((id) => ({
      id: `surface:${id}`,
      label: id === "chat" ? t("チャット") : t("エージェント"),
      hint: t("サーフェスを切り替え"),
      icon: id === "chat" ? <MessageSquare size={15} /> : <Bot size={15} />,
      kind: "surface",
      run: () => setSurface(id),
    }));
    const modelItems: PaletteItem[] = (models.data ?? [])
      .slice(0, 30)
      .map((m) => ({
        id: `model:${m.id}`,
        label: m.label,
        hint: t("モデル"),
        icon: <Sparkles size={15} />,
        kind: "model",
        run: () => {
          setModelId(m.id);
          setView("chat");
        },
      }));
    return [
      {
        id: "action:new-thread",
        label: t("新規スレッド"),
        keys: "⌘N",
        icon: <Plus size={15} />,
        kind: "action",
        run: () => {
          setView("chat");
          window.dispatchEvent(new CustomEvent("mix:new-thread"));
        },
      },
      {
        id: "action:toggle-sidebar",
        label: t("サイドバー切り替え"),
        keys: "⌘B",
        icon: <PanelLeft size={15} />,
        kind: "action",
        run: () => toggleSidebar(),
      },
      {
        id: "action:toggle-file-tree",
        label: t("ファイルツリー表示切替"),
        keys: "⌘⇧E",
        icon: <FolderTree size={15} />,
        kind: "action",
        run: () => toggleFileTree(),
      },
      {
        id: "action:settings",
        label: t("設定を開く"),
        keys: "⌘,",
        icon: <Settings2 size={15} />,
        kind: "action",
        run: () => openSettings(),
      },
      {
        id: "action:shortcuts",
        label: t("ショートカット一覧"),
        keys: "?",
        icon: <Keyboard size={15} />,
        kind: "action",
        run: () => setShortcutsOpen(true),
      },
      ...surfaces,
      ...views,
      ...settingsTabs,
      ...modelItems,
      ...threads,
    ];
  }, [
    conversations.data,
    models.data,
    openSettings,
    setConversationId,
    setModelId,
    setView,
    setSurface,
    setShortcutsOpen,
    toggleSidebar,
    toggleFileTree,
    locale,
  ]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items.slice(0, 12);
    // Substring matches rank first, subsequence fuzzy matches follow — a
    // short typo-tolerant query still finds the row instead of an empty list.
    const exact: PaletteItem[] = [];
    const fuzzy: PaletteItem[] = [];
    for (const item of items) {
      const label = item.label.toLowerCase();
      if (label.includes(q)) exact.push(item);
      else if (fuzzyMatch(label, q)) fuzzy.push(item);
      if (exact.length + fuzzy.length >= 20) break;
    }
    return [...exact, ...fuzzy].slice(0, 20);
  }, [items, query]);

  useEffect(() => {
    setSelected(0);
  }, [query]);

  // Keep the keyboard-highlighted row inside the scroll viewport. Guarded for
  // jsdom, where `scrollIntoView` is not implemented.
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    listRef.current
      ?.querySelector<HTMLElement>(`#palette-option-${selected}`)
      ?.scrollIntoView?.({ block: "nearest" });
  }, [selected, open]);

  if (!open) return null;

  // Pending → empty: while the lists load the rows would otherwise render the
  // "no match" message for a list that simply has not arrived yet.
  const pending = conversations.isPending || models.isPending;

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
          <Search
            size={15}
            className="shrink-0 text-faint"
            aria-hidden="true"
          />
          <input
            // APG dialog pattern: the palette opens for typing; focus must
            // start in the query, not on the body behind it.
            // eslint-disable-next-line jsx-a11y/no-autofocus
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
          ref={listRef}
          role="listbox"
          aria-label={t("コマンドパレット")}
          className="flex min-h-0 flex-col gap-0.5 overflow-y-auto p-1.5"
        >
          {pending ? (
            <ListboxSkeleton />
          ) : (
            <>
              {filtered.map((item, i) => (
                <ListRow
                  role="option"
                  id={`palette-option-${i}`}
                  aria-selected={i === selected}
                  key={item.id}
                  selected={i === selected}
                  onMouseEnter={() => setSelected(i)}
                  onFocus={() => setSelected(i)}
                  onClick={() => {
                    item.run();
                    setOpen(false);
                  }}
                >
                  <span className="shrink-0 text-faint" aria-hidden="true">
                    {item.icon}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-sm">
                    {item.label}
                  </span>
                  {item.keys ? (
                    <Kbd className="ms-auto shrink-0">{item.keys}</Kbd>
                  ) : item.hint ? (
                    <span className="ms-auto shrink-0 text-xs text-faint">
                      {item.hint}
                    </span>
                  ) : (
                    <span className="ms-auto shrink-0 text-2xs text-faint/70">
                      {t(KIND_LABEL[item.kind])}
                    </span>
                  )}
                  {i === selected && (
                    <CornerDownLeft
                      size={12}
                      className="shrink-0 text-faint"
                      aria-hidden="true"
                    />
                  )}
                </ListRow>
              ))}
              {filtered.length === 0 && (
                <EmptyState
                  compact
                  title={t("一致する項目がありません")}
                  sub={
                    query.trim()
                      ? t("別のキーワードで検索してください")
                      : undefined
                  }
                />
              )}
            </>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-3 border-t border-line bg-surface/60 px-3 py-1.5 text-2xs text-faint">
          <span className="inline-flex items-center gap-1" aria-hidden="true">
            <Kbd>↑↓</Kbd> {t("移動")}
          </span>
          <span className="inline-flex items-center gap-1" aria-hidden="true">
            <Kbd>Enter</Kbd> {t("開く")}
          </span>
          <span className="inline-flex items-center gap-1" aria-hidden="true">
            <Kbd>Esc</Kbd> {t("閉じる")}
          </span>
          <span className="ms-auto font-mono tabular-nums">
            {filtered.length} {t("件")}
          </span>
        </div>
      </DialogContent>
    </Dialog>
  );
}
