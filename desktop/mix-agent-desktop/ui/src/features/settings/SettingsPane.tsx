import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Archive, BookOpen, Brain, Globe, Globe2, Layers, Palette, RefreshCw, Save,
  Settings2, ShieldCheck, Sparkles, Terminal, Wrench, Zap,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useSettingsForm } from "@/lib/settings";
import { usePendingApprovals } from "@/lib/queries";
import { useUi, type SettingsTab } from "@/lib/ui";
import type { SettingsView } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { ErrorAlert } from "@/components/shared";
import { Button } from "@/components/ui/button";
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
  "appearance", "general", "network", "updates", "backup",
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
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["settings"] }),
  });
  const pendingCount = approvals.data?.length ?? 0;
  const showSave = SAVE_TABS.has(activeTab);
  // Saving before `get_settings` resolves would replace the stored row with
  // the defaults rendered in the form.
  const canSave = loaded && !settings.isError && !update.isPending;

  return (
    <div className="flex min-h-0 flex-1">
      <aside
        className="flex w-56 shrink-0 flex-col gap-4 overflow-y-auto border-e border-line bg-sidebar p-2.5"
        aria-label={t("設定")}
      >
        <div className="flex items-center gap-1.5 px-1.5 pt-1 text-sm font-semibold text-fg">
          <Settings2 size={16} className="text-faint" />
          {t("設定")}
        </div>
        {SETTINGS_NAV.map((section) => (
          <nav key={section.title} className="flex flex-col gap-0.5">
            <h2 className="px-1.5 pb-1 text-2xs font-semibold uppercase tracking-wide text-faint">
              {t(section.title)}
            </h2>
            {section.tabs.map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setSettingsTab(tab.id)}
                aria-current={activeTab === tab.id ? "page" : undefined}
                className={cn(
                  "flex h-7 w-full items-center gap-2 rounded-md px-2 text-sm",
                  "text-muted transition-colors duration-100 hover:bg-hover hover:text-fg",
                  "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                  activeTab === tab.id && "bg-active font-medium text-fg",
                )}
              >
                <span className="shrink-0 text-faint">{tab.icon}</span>
                <span className="min-w-0 flex-1 truncate text-start">{t(tab.label)}</span>
                {tab.id === "approvals" && pendingCount > 0 && (
                  <Badge tone="warning" size="sm">{pendingCount}</Badge>
                )}
              </button>
            ))}
          </nav>
        ))}
      </aside>

      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex h-14 shrink-0 items-center justify-between gap-3 border-b border-line px-6">
          <h2 className="text-lg font-semibold tracking-tight text-fg">
            {t(ALL_TABS.find((tab) => tab.id === activeTab)?.label ?? "設定")}
          </h2>
          <div className="flex items-center gap-2">
            {showSave && update.isSuccess && !update.isError && (
              <Badge tone="success">{t("保存しました")}</Badge>
            )}
            {showSave && (
              <Button
                variant="primary"
                disabled={!canSave}
                onClick={() => update.mutate(data)}
              >
                <Save size={12} /> {t("保存")}
              </Button>
            )}
          </div>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-6">
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
  );
}
