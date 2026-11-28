import { useEffect, useRef, useState } from "react";
import {
  Archive,
  BookOpen,
  Brain,
  Globe2,
  Layers,
  Palette,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Terminal,
  Wrench,
  Zap,
} from "lucide-react";
import { t } from "@/lib/i18n";
import { useSettingsForm, useSettingsSaveStatus } from "@/lib/settings";
import { useUi, type SettingsTab } from "@/lib/ui";
import { Badge } from "@/components/ui/badge";
import { ErrorAlert } from "@/components/shared";
import { NavRow, SectionLabel } from "@/components/layout";
import { SearchField, Select } from "@/components/ui/input";
import { AppearanceSettings } from "./AppearanceSettings";
import { GeneralSettings } from "./GeneralSettings";
import { UpdateSettings } from "./UpdateSettings";
import { BackupSettings } from "./BackupSettings";
import { ProvidersPane } from "@/features/providers/ProvidersPane";
import { ModelsPane } from "@/features/providers/ModelsPane";
import { ToolsPane } from "@/features/tools/ToolsPane";
import { McpPane } from "@/features/mcp/McpPane";
import { MemoryPane } from "@/features/memory/MemoryPane";
import { KnowledgePane } from "@/features/knowledge/KnowledgePane";
import { SkillsPane } from "@/features/skills/SkillsPane";
import { BrowserPane } from "@/features/browser/BrowserPane";
import { isMacPlatform } from "@/lib/platform";
import {
  matchSettingsAnchors,
  type SettingsAnchor,
} from "@/lib/settingsAnchors";
import { DiagnosticsPane } from "@/features/system/DiagnosticsPane";
import { SpotlightSettings } from "./SpotlightSettings";

interface SettingsTabEntry {
  id: SettingsTab;
  label: string;
  icon: React.ReactNode;
  /** Extra terms the settings search should match (API key, localhost, …). */
  keywords?: readonly string[];
}

/**
 * Settings navigation grouped by what the user is trying to do, not by how the
 * backend is organised. The previous three groups ("アプリ / 構成 / ナレッジ")
 * mixed model setup with backups and pushed 17 equally-weighted tabs; finding
 * "where do I connect a model?" meant scanning the whole list.
 */
const SETTINGS_NAV: { title: string; tabs: SettingsTabEntry[] }[] = [
  {
    title: "基本",
    tabs: [
      {
        id: "general",
        label: "一般",
        icon: <Zap size={15} />,
        keywords: [
          "スタートアップ",
          "自動起動",
          "プロンプト履歴",
          "下書き",
          "予算",
          "承認期限",
          "承認",
          "サンドボックス",
          "検証コマンド",
          "フォーマット",
          "暗号化",
          "データベース",
          "ネットワーク",
          "localhost",
          "egress",
          "許可リスト",
          "api",
        ],
      },
      {
        id: "appearance",
        label: "外観",
        icon: <Palette size={15} />,
        keywords: [
          "テーマ",
          "ダーク",
          "ライト",
          "言語",
          "文字",
          "コード折り返し",
        ],
      },
    ],
  },
  {
    title: "モデルと接続",
    tabs: [
      {
        id: "providers",
        label: "プロバイダー",
        icon: <Layers size={15} />,
        keywords: [
          "apiキー",
          "api key",
          "base url",
          "接続",
          "ollama",
          "lm studio",
          "ローカル",
          "chatgpt",
          "codex",
        ],
      },
      {
        id: "models",
        label: "モデル",
        icon: <Sparkles size={15} />,
        keywords: ["モデル一覧", "既定モデル", "vision", "コンテキスト"],
      },
    ],
  },
  {
    title: "エージェント",
    tabs: [
      {
        id: "tools",
        label: "ツール",
        icon: <Wrench size={15} />,
        keywords: ["ツール", "許可", "実行", "ブラウザ", "ターミナル"],
      },
      {
        id: "mcp",
        label: "MCP",
        icon: <Globe2 size={15} />,
        keywords: ["mcp", "サーバー", "oauth", "接続"],
      },
      {
        id: "skills",
        label: "スキル",
        icon: <Terminal size={15} />,
        keywords: ["スキル", "skill", "個人", "発見"],
      },
    ],
  },
  {
    title: "データと検索",
    tabs: [
      {
        id: "memory",
        label: "メモリ",
        icon: <Brain size={15} />,
        keywords: ["メモリ", "記憶", "自動形成", "埋め込み"],
      },
      {
        id: "knowledge",
        label: "ナレッジ",
        icon: <BookOpen size={15} />,
        keywords: ["ナレッジ", "文書", "検索", "インデックス"],
      },
      {
        id: "browser",
        label: "ブラウザ",
        icon: <Globe2 size={15} />,
        keywords: ["ブラウザ", "セッション", "ダウンロード", "プロファイル"],
      },
    ],
  },
  {
    title: "アプリ",
    tabs: [
      {
        id: "updates",
        label: "アップデート",
        icon: <RefreshCw size={15} />,
        keywords: ["更新", "バージョン", "自動チェック", "ダウンロード"],
      },
      {
        id: "backup",
        label: "バックアップ",
        icon: <Archive size={15} />,
        keywords: ["バックアップ", "復元", "エクスポート", "インポート"],
      },
    ],
  },
  {
    title: "詳細・診断",
    tabs: [
      {
        id: "spotlight",
        label: "Spotlight",
        icon: <Search size={15} />,
        keywords: ["spotlight", "macos", "システム検索", "索引"],
      },
      {
        id: "diagnostics",
        label: "診断",
        icon: <ShieldCheck size={15} />,
        keywords: ["診断", "ログ", "uxイベント", "パフォーマンス", "ヘルス"],
      },
    ],
  },
];

const ALL_TABS = SETTINGS_NAV.flatMap((s) => s.tabs);

/** "予算" etc. → the exact tab and card, instead of only filtering the nav. */
function AnchorMatches({
  matches,
  onSelect,
}: {
  matches: readonly SettingsAnchor[];
  onSelect: (anchor: SettingsAnchor) => void;
}) {
  if (matches.length === 0) return null;
  return (
    <nav className="flex flex-col gap-0.5" aria-label={t("設定項目")}>
      <SectionLabel>{t("設定項目")}</SectionLabel>
      {matches.map((anchor) => {
        const tab = ALL_TABS.find((entry) => entry.id === anchor.tab);
        return (
          <button
            key={anchor.id}
            type="button"
            onClick={() => onSelect(anchor)}
            className="flex min-w-0 items-center gap-2 rounded-md px-2.5 py-1.5 text-start transition-colors hover:bg-hover focus-visible:bg-hover"
            title={tab ? `${t(tab.label)} › ${t(anchor.label)}` : undefined}
          >
            <span className="min-w-0 flex-1 truncate text-xs text-fg">
              {t(anchor.label)}
            </span>
            <span className="shrink-0 text-2xs text-faint">
              {tab ? t(tab.label) : ""}
            </span>
          </button>
        );
      })}
    </nav>
  );
}

export function SettingsPane() {
  const activeTab = useUi((s) => s.settingsTab);
  const setSettingsTab = useUi((s) => s.setSettingsTab);
  // Every tab writes through `useSettingsAutosave` (mounted at the app root).
  // This pane only shows the writer's status and a load failure.
  const { query: settings } = useSettingsForm();
  const saveStatus = useSettingsSaveStatus((s) => s.status);
  const [filter, setFilter] = useState("");
  const [focusTarget, setFocusTarget] = useState<{
    tab: SettingsTab;
    phrase: string;
    /** Allows re-selecting the same result to scroll again. */
    nonce: number;
  } | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const flashTimer = useRef<number | null>(null);
  const query = filter.trim().toLowerCase();
  const anchorMatches = matchSettingsAnchors(filter, t);
  const matchesTab = (tab: SettingsTabEntry) =>
    !query ||
    t(tab.label).toLowerCase().includes(query) ||
    tab.label.toLowerCase().includes(query) ||
    (tab.keywords ?? []).some((keyword) =>
      keyword.toLowerCase().includes(query),
    );
  const visibleSections = SETTINGS_NAV.map((section) => ({
    ...section,
    tabs: section.tabs.filter(
      (tab) => (tab.id !== "spotlight" || isMacPlatform()) && matchesTab(tab),
    ),
  })).filter((section) => section.tabs.length > 0);
  const visibleTabs = visibleSections.flatMap((section) => section.tabs);
  const platformTabs = ALL_TABS.filter(
    (tab) => tab.id !== "spotlight" || isMacPlatform(),
  );

  const goToAnchor = (anchor: SettingsAnchor) => {
    setSettingsTab(anchor.tab);
    setFilter("");
    setFocusTarget({
      tab: anchor.tab,
      phrase: t(anchor.label),
      nonce: Date.now(),
    });
  };

  // Scroll after the target tab has rendered. Two frames: the first lets the
  // tab switch commit, the second lets its cards lay out before measuring.
  useEffect(() => {
    if (!focusTarget || activeTab !== focusTarget.tab) return;
    let second = 0;
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() => {
        const root = contentRef.current;
        if (!root) return;
        const phrase = focusTarget.phrase.toLowerCase();
        // Prefer the shortest matching text node: a section's own textContent
        // contains every child, so the naive first match would always be the
        // outer page and scroll nowhere useful.
        const candidates = Array.from(
          root.querySelectorAll<HTMLElement>("h3,label,span,p,button"),
        ).filter((el) => {
          const text = (el.textContent ?? "").toLowerCase();
          return text.includes(phrase) && text.length <= phrase.length + 80;
        });
        const target = candidates.sort(
          (a, b) =>
            (a.textContent?.length ?? 0) - (b.textContent?.length ?? 0),
        )[0];
        const section = target?.closest("section") ?? target;
        if (!section) return;
        section.scrollIntoView?.({ behavior: "smooth", block: "center" });
        section.classList.remove("settings-flash");
        void section.offsetWidth;
        section.classList.add("settings-flash");
        if (flashTimer.current !== null) {
          window.clearTimeout(flashTimer.current);
        }
        flashTimer.current = window.setTimeout(() => {
          section.classList.remove("settings-flash");
          flashTimer.current = null;
        }, 1800);
      });
    });
    return () => {
      cancelAnimationFrame(first);
      if (second) cancelAnimationFrame(second);
    };
  }, [focusTarget, activeTab]);

  useEffect(
    () => () => {
      if (flashTimer.current !== null) window.clearTimeout(flashTimer.current);
    },
    [],
  );

  return (
    <div className="flex min-h-0 flex-1">
      <aside
        className="hidden w-56 shrink-0 flex-col gap-4 overflow-y-auto border-e border-line bg-sidebar p-2.5 md:flex"
        aria-label={t("設定")}
      >
        <div className="flex items-center gap-1.5 px-1.5 pt-1 text-sm font-semibold text-fg">
          <Settings2 size={16} className="text-faint" />
          {t("設定")}
        </div>
        <SearchField
          icon={<Search size={13} />}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("設定を検索")}
          aria-label={t("設定を検索")}
          {...(filter
            ? { onClear: () => setFilter(""), clearLabel: t("クリア") }
            : {})}
        />
        <AnchorMatches matches={anchorMatches} onSelect={goToAnchor} />
        {visibleSections.length === 0 && anchorMatches.length === 0 && (
          <p className="px-1.5 py-2 text-xs text-faint">
            {t("一致する設定がありません")}
          </p>
        )}
        {visibleSections.map((section) => (
          <nav key={section.title} className="flex flex-col gap-0.5">
            <SectionLabel>{t(section.title)}</SectionLabel>
            {section.tabs.map((tab) => (
              <NavRow
                key={tab.id}
                icon={tab.icon}
                selected={activeTab === tab.id}
                onClick={() => setSettingsTab(tab.id)}
              >
                {t(tab.label)}
              </NavRow>
            ))}
          </nav>
        ))}
      </aside>

      {/* `min-w-0` is load-bearing: without it this column refuses to shrink
          below its content's min-content width, so a wide DataTable (providers,
          models, tools, MCP) pushed the whole pane past the window edge. */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {/* Matches `Page`'s header (sticky, `max-w-content` measure) so the
            title lines up with the cards below on wide windows. The wrapping
            row and the `md:hidden` select keep the narrow-screen behaviour. */}
        <header className="sticky top-0 z-10 flex shrink-0 flex-wrap items-center gap-3 border-b border-line bg-bg/85 px-6 py-3.5 backdrop-blur-md">
          <div className="mx-auto flex w-full max-w-content items-center gap-3">
            <div className="min-w-0 flex-1">
              <h1 className="truncate text-base font-semibold tracking-tight text-fg">
                {t(
                  platformTabs.find((tab) => tab.id === activeTab)?.label ??
                    "設定",
                )}
              </h1>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <span className="hidden text-2xs text-faint sm:inline">
                {t("変更は自動保存されます")}
              </span>
              {saveStatus === "saving" && (
                <Badge tone="warning">{t("保存中…")}</Badge>
              )}
              {saveStatus === "saved" && (
                <Badge tone="success">{t("保存しました")}</Badge>
              )}
              {saveStatus === "error" && (
                <Badge tone="danger">{t("保存できませんでした")}</Badge>
              )}
            </div>
          </div>
          {/* Narrow screens hide the sidebar: the search field and select take
              over section switching. Without the search here the settings
              search disappeared entirely below `md`. */}
          <div className="mx-auto flex w-full max-w-content flex-col gap-2 md:hidden">
            <SearchField
              icon={<Search size={13} />}
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder={t("設定を検索")}
              aria-label={t("設定を検索")}
              {...(filter
                ? { onClear: () => setFilter(""), clearLabel: t("クリア") }
                : {})}
            />
            <AnchorMatches matches={anchorMatches} onSelect={goToAnchor} />
            <Select
              aria-label={t("設定セクションを選択")}
              value={activeTab}
              onChange={(e) => setSettingsTab(e.target.value as SettingsTab)}
            >
              {(visibleTabs.length > 0 ? visibleTabs : platformTabs).map(
                (tab) => (
                  <option key={tab.id} value={tab.id}>
                    {t(tab.label)}
                  </option>
                ),
              )}
            </Select>
          </div>
        </header>

        {/* Same content measure as every other top-level pane. This body used
            to be full-bleed, so settings cards were the only thing in the app
            that stretched to the window edges. */}
        <div
          ref={contentRef}
          className="flex min-h-0 flex-1 flex-col overflow-y-auto"
        >
          <div className="mx-auto flex w-full max-w-content flex-col gap-4 px-6 py-6">
            {/* A failed `get_settings` used to leave every control silently on
                its default while Save stayed disabled with no explanation. */}
            {settings.isError && <ErrorAlert error={settings.error} />}

            {activeTab === "appearance" && <AppearanceSettings />}
            {activeTab === "general" && <GeneralSettings />}
            {activeTab === "updates" && <UpdateSettings />}
            {activeTab === "backup" && <BackupSettings />}
            {activeTab === "spotlight" && <SpotlightSettings />}
            {activeTab === "providers" && <ProvidersPane />}
            {activeTab === "models" && <ModelsPane />}
            {activeTab === "tools" && <ToolsPane />}
            {activeTab === "mcp" && <McpPane />}
            {activeTab === "memory" && <MemoryPane />}
            {activeTab === "knowledge" && <KnowledgePane />}
            {activeTab === "skills" && <SkillsPane />}
            {activeTab === "browser" && <BrowserPane />}
            {activeTab === "diagnostics" && <DiagnosticsPane />}
          </div>
        </div>
      </div>
    </div>
  );
}
