import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useEffect } from "react";
import {
  Archive,
  BookOpen,
  Brain,
  Globe,
  Globe2,
  Layers,
  Palette,
  RefreshCw,
  Save,
  Settings2,
  ShieldCheck,
  Sparkles,
  Terminal,
  Wrench,
  Zap,
} from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useSettingsForm } from "@/lib/settings";
import { usePendingApprovals } from "@/lib/queries";
import { useUi, type SettingsTab } from "@/lib/ui";
import type { SettingsView } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { ErrorAlert } from "@/components/shared";
import { NavRow, SectionLabel } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/input";
import { AppearanceSettings } from "./AppearanceSettings";
import { GeneralSettings } from "./GeneralSettings";
import { NetworkSettings } from "./NetworkSettings";
import { UpdateSettings } from "./UpdateSettings";
import { BackupSettings } from "./BackupSettings";
import { ProvidersPane } from "@/features/providers/ProvidersPane";
import { ModelsPane } from "@/features/providers/ModelsPane";
import { ToolsPane } from "@/features/tools/ToolsPane";
import { ApprovalsPane } from "@/features/tools/ApprovalsPane";
import { McpPane } from "@/features/mcp/McpPane";
import { MemoryPane } from "@/features/memory/MemoryPane";
import { KnowledgePane } from "@/features/knowledge/KnowledgePane";
import { SkillsPane } from "@/features/skills/SkillsPane";
import { BrowserPane } from "@/features/browser/BrowserPane";
import { DiagnosticsPane } from "@/features/system/DiagnosticsPane";

interface SettingsTabEntry {
  id: SettingsTab;
  label: string;
  icon: React.ReactNode;
}

const SETTINGS_NAV: { title: string; tabs: SettingsTabEntry[] }[] = [
  {
    title: "アプリ",
    tabs: [
      { id: "appearance", label: "外観", icon: <Palette size={15} /> },
      { id: "general", label: "一般", icon: <Zap size={15} /> },
      { id: "network", label: "ネットワーク", icon: <Globe size={15} /> },
      { id: "updates", label: "アップデート", icon: <RefreshCw size={15} /> },
      { id: "backup", label: "バックアップ", icon: <Archive size={15} /> },
      { id: "diagnostics", label: "診断", icon: <ShieldCheck size={15} /> },
    ],
  },
  {
    title: "構成",
    tabs: [
      { id: "providers", label: "プロバイダー", icon: <Layers size={15} /> },
      { id: "models", label: "モデル", icon: <Sparkles size={15} /> },
      { id: "tools", label: "ツール", icon: <Wrench size={15} /> },
      { id: "approvals", label: "承認", icon: <ShieldCheck size={15} /> },
      { id: "mcp", label: "MCP", icon: <Globe2 size={15} /> },
      { id: "browser", label: "ブラウザ", icon: <Globe size={15} /> },
    ],
  },
  {
    title: "ナレッジ",
    tabs: [
      { id: "memory", label: "メモリ", icon: <Brain size={15} /> },
      { id: "knowledge", label: "ナレッジ", icon: <BookOpen size={15} /> },
      { id: "skills", label: "スキル", icon: <Terminal size={15} /> },
    ],
  },
];

/** Tabs that edit the persisted SettingsView and share the header Save button. */
const SAVE_TABS: ReadonlySet<SettingsTab> = new Set([
  "appearance",
  "general",
  "network",
  "updates",
  "backup",
]);

const ALL_TABS = SETTINGS_NAV.flatMap((s) => s.tabs);

export function SettingsPane() {
  const activeTab = useUi((s) => s.settingsTab);
  const setSettingsTab = useUi((s) => s.setSettingsTab);
  const queryClient = useQueryClient();
  const { data, loaded, query: settings } = useSettingsForm();
  const approvals = usePendingApprovals();
  const update = useMutation({
    mutationFn: (input: SettingsView) => api.updateSettings(input),
    onSuccess: (_res, input) => {
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      setSavedSnapshot(JSON.stringify(input));
    },
  });
  const pendingCount = approvals.data?.length ?? 0;
  const showSave = SAVE_TABS.has(activeTab);
  // Snapshot of the draft as it was when Save last succeeded. Without it the
  // "保存しました" badge stayed lit forever: editing afterwards still showed a
  // saved state the moment before the user changed the value.
  const [savedSnapshot, setSavedSnapshot] = useState<string | null>(null);
  const dirty =
    savedSnapshot !== null && JSON.stringify(data) !== savedSnapshot;
  // Saving before `get_settings` resolves would replace the stored row with
  // the defaults rendered in the form.
  const canSave = loaded && !settings.isError && !update.isPending;
  // Baseline the loaded form so "未保存" is not shown before the user edits.
  useEffect(() => {
    if (!loaded || savedSnapshot !== null) return;
    setSavedSnapshot(JSON.stringify(data));
  }, [data, loaded, savedSnapshot]);
  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

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
        {SETTINGS_NAV.map((section) => (
          <nav key={section.title} className="flex flex-col gap-0.5">
            <SectionLabel>{t(section.title)}</SectionLabel>
            {section.tabs.map((tab) => (
              <NavRow
                key={tab.id}
                icon={tab.icon}
                selected={activeTab === tab.id}
                onClick={() => setSettingsTab(tab.id)}
                trailing={
                  tab.id === "approvals" && pendingCount > 0 ? (
                    <Badge tone="warning" size="sm" className="font-mono">
                      {pendingCount}
                    </Badge>
                  ) : undefined
                }
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
                  ALL_TABS.find((tab) => tab.id === activeTab)?.label ?? "設定",
                )}
              </h1>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {showSave && update.isSuccess && !update.isError && !dirty && (
                <Badge tone="success">{t("保存しました")}</Badge>
              )}
              {showSave && dirty && !update.isPending && (
                <Badge tone="warning">{t("未保存の変更")}</Badge>
              )}
              {showSave && (
                <Button
                  variant="primary"
                  disabled={!canSave || !dirty}
                  onClick={() => update.mutate(data)}
                >
                  <Save size={12} /> {t("保存")}
                </Button>
              )}
            </div>
          </div>
          {/* Narrow screens hide the sidebar: this select takes over section
              switching. `md:hidden` keeps it out of the way once the aside is
              back, and the wrapping header gives it its own row. */}
          <div className="mx-auto w-full max-w-content md:hidden">
            <Select
              aria-label={t("設定セクションを選択")}
              value={activeTab}
              onChange={(e) => setSettingsTab(e.target.value as SettingsTab)}
            >
              {ALL_TABS.map((tab) => (
                <option key={tab.id} value={tab.id}>
                  {t(tab.label)}
                </option>
              ))}
            </Select>
          </div>
        </header>

        {/* Same content measure as every other top-level pane. This body used
            to be full-bleed, so settings cards were the only thing in the app
            that stretched to the window edges. */}
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          <div className="mx-auto flex w-full max-w-content flex-col gap-4 px-6 py-6">
            {/* A failed `get_settings` used to leave every control silently on
                its default while Save stayed disabled with no explanation. */}
            {settings.isError && <ErrorAlert error={settings.error} />}
            {showSave && update.isError && <ErrorAlert error={update.error} />}

            {activeTab === "appearance" && <AppearanceSettings />}
            {activeTab === "general" && <GeneralSettings />}
            {activeTab === "network" && <NetworkSettings />}
            {activeTab === "updates" && <UpdateSettings />}
            {activeTab === "backup" && <BackupSettings />}
            {activeTab === "providers" && <ProvidersPane />}
            {activeTab === "models" && <ModelsPane />}
            {activeTab === "tools" && <ToolsPane />}
            {activeTab === "approvals" && <ApprovalsPane />}
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
