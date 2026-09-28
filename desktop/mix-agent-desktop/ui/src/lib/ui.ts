import { create } from "zustand";
import { getLanguage, setLanguage, type Language } from "./i18n";
import { applyTheme, storeTheme, type Theme } from "./theme";

export type View = "chat" | "agents" | "projects" | "schedules" | "settings";

/** Tabs inside the Settings pane (see features/settings/SettingsPane.tsx). */
export type SettingsTab =
  | "appearance" | "general" | "network" | "updates" | "backup"
  | "providers" | "models" | "tools" | "approvals" | "mcp"
  | "memory" | "knowledge" | "skills" | "browser" | "diagnostics";

export type RightPanel = "none" | "activity" | "diff" | "changes" | "file";

export interface DiffTarget {
  /** Workspace-relative path the `files` tool reported. */
  path: string;
  /** Proposed file content from the tool arguments. */
  content: string;
  label: string;
}

interface UiState {
  view: View;
  setView: (view: View) => void;
  /** Active tab inside the Settings pane, shared so ⌘K / menu can target it. */
  settingsTab: SettingsTab;
  setSettingsTab: (tab: SettingsTab) => void;
  /** Navigate to the Settings pane, optionally opening a specific tab. */
  openSettings: (tab?: SettingsTab) => void;
  conversationId: string | null;
  setConversationId: (id: string | null) => void;
  runId: string | null;
  setRunId: (id: string | null) => void;
  modelId: string;
  setModelId: (model: string) => void;
  mode: "chat" | "thinking" | "agent";
  setMode: (mode: "chat" | "thinking" | "agent") => void;
  sidebarOpen: boolean;
  toggleSidebar: () => void;
  fileTreeOpen: boolean;
  toggleFileTree: () => void;
  /** Project whose folder drives the file tree / diff scope. */
  activeProjectId: string | null;
  setActiveProjectId: (id: string | null) => void;
  rightPanel: RightPanel;
  setRightPanel: (panel: RightPanel) => void;
  /** Run whose activity is shown in the right panel (live run or history). */
  activityRunId: string | null;
  setActivityRunId: (id: string | null) => void;
  diffTarget: DiffTarget | null;
  openDiff: (target: DiffTarget) => void;
  /** Root-relative path shown in the read-only file preview. */
  filePreviewPath: string | null;
  openFilePreview: (path: string) => void;
  /** Git status entry whose diff is shown (set from the changes list). */
  gitDiffPath: string | null;
  setGitDiffPath: (path: string | null) => void;
  locale: Language;
  setLocale: (locale: Language) => void;
  theme: Theme;
  setTheme: (theme: Theme) => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
}

const SIDEBAR_KEY = "mix-agent-sidebar-open";
const RIGHT_PANEL_KEY = "mix-agent-right-panel";
const FILE_TREE_KEY = "mix-agent-file-tree-open";
const ACTIVE_PROJECT_KEY = "mix-agent-active-project";
const SETTINGS_TAB_KEY = "mix-agent-settings-tab";

function readActiveProject(): string | null {
  try {
    return localStorage.getItem(ACTIVE_PROJECT_KEY);
  } catch {
    return null;
  }
}

function readBoolean(key: string, fallback: boolean): boolean {
  try {
    const stored = localStorage.getItem(key);
    if (stored === null) return fallback;
    return stored === "1";
  } catch {
    return fallback;
  }
}

function readRightPanel(): RightPanel {
  try {
    const stored = localStorage.getItem(RIGHT_PANEL_KEY);
    if (
      stored === "none" || stored === "activity" || stored === "diff"
      || stored === "changes" || stored === "file"
    ) {
      return stored;
    }
  } catch {
    // fall through
  }
  return "activity";
}

const SETTING_TABS: SettingsTab[] = [
  "appearance", "general", "network", "updates", "backup",
  "providers", "models", "tools", "approvals", "mcp",
  "memory", "knowledge", "skills", "browser", "diagnostics",
];

function readSettingsTab(): SettingsTab {
  try {
    const stored = localStorage.getItem(SETTINGS_TAB_KEY);
    if (stored && SETTING_TABS.includes(stored as SettingsTab)) return stored as SettingsTab;
  } catch {
    // fall through
  }
  return "appearance";
}

export const useUi = create<UiState>((set) => ({
  view: "chat",
  setView: (view) => set({ view }),
  settingsTab: readSettingsTab(),
  setSettingsTab: (settingsTab) => {
    try {
      localStorage.setItem(SETTINGS_TAB_KEY, settingsTab);
    } catch {
      // best-effort
    }
    set({ settingsTab });
  },
  openSettings: (tab) =>
    set((state) => ({
      view: "settings",
      settingsTab: tab ?? state.settingsTab,
    })),
  conversationId: null,
  setConversationId: (id) => set({ conversationId: id }),
  runId: null,
  setRunId: (id) => set({ runId: id }),
  modelId: "",
  setModelId: (model) => set({ modelId: model }),
  mode: "chat",
  setMode: (mode) => set({ mode }),
  sidebarOpen: readBoolean(SIDEBAR_KEY, true),
  toggleSidebar: () =>
    set((state) => {
      const sidebarOpen = !state.sidebarOpen;
      try {
        localStorage.setItem(SIDEBAR_KEY, sidebarOpen ? "1" : "0");
      } catch {
        // best-effort
      }
      return { sidebarOpen };
    }),
  fileTreeOpen: readBoolean(FILE_TREE_KEY, false),
  toggleFileTree: () =>
    set((state) => {
      const fileTreeOpen = !state.fileTreeOpen;
      try {
        localStorage.setItem(FILE_TREE_KEY, fileTreeOpen ? "1" : "0");
      } catch {
        // best-effort
      }
      return { fileTreeOpen };
    }),
  activeProjectId: readActiveProject(),
  setActiveProjectId: (activeProjectId) => {
    try {
      if (activeProjectId === null) localStorage.removeItem(ACTIVE_PROJECT_KEY);
      else localStorage.setItem(ACTIVE_PROJECT_KEY, activeProjectId);
    } catch {
      // best-effort
    }
    set({ activeProjectId });
  },
  rightPanel: readRightPanel(),
  setRightPanel: (rightPanel) => {
    try {
      localStorage.setItem(RIGHT_PANEL_KEY, rightPanel);
    } catch {
      // best-effort
    }
    set({ rightPanel });
  },
  activityRunId: null,
  setActivityRunId: (activityRunId) => set({ activityRunId }),
  diffTarget: null,
  openDiff: (diffTarget) => set({ diffTarget, rightPanel: "diff" }),
  filePreviewPath: null,
  openFilePreview: (filePreviewPath) => set({ filePreviewPath, rightPanel: "file" }),
  gitDiffPath: null,
  setGitDiffPath: (gitDiffPath) => set({ gitDiffPath }),
  locale: getLanguage(),
  setLocale: (locale) => {
    setLanguage(locale);
    set({ locale });
  },
  theme: "system",
  setTheme: (theme) => {
    storeTheme(theme);
    set({ theme });
  },
  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
}));

/** Apply the stored theme once at boot (before React renders). */
export function initTheme(theme: Theme): void {
  applyTheme(theme);
  useUi.setState({ theme });
}
