import { create } from "zustand";
import { getLanguage, setLanguage, type Language } from "./i18n";
import { storeTheme, type Theme } from "./theme";

export type View = "chat" | "agents" | "projects" | "schedules" | "settings";

/**
 * Top-level UI surface the user is currently in.
 * - `chat`          — Simple Q&A experience (chat / thinking only, no workspace chrome)
 * - `desktop-agent` — Agentic workspace (agent mode, file tree, changes, activity)
 */
export type Surface = "chat" | "desktop-agent";

/** Run mode understood by the Rust Run Engine (see `core/src/mode.rs`). */
export type RunMode = "chat" | "thinking" | "agent";

/** Filter applied to the sidebar thread list. */
export type ThreadFilter = "all" | Surface;

/** Modes a surface allows the user to pick from in the Composer. */
export const SURFACE_MODES: Record<Surface, RunMode[]> = {
  chat: ["chat", "thinking"],
  "desktop-agent": ["agent"],
};

/** Tabs inside the Settings pane (see features/settings/SettingsPane.tsx). */
export type SettingsTab =
  | "appearance" | "general" | "network" | "updates" | "backup"
  | "providers" | "models" | "tools" | "approvals" | "mcp"
  | "memory" | "knowledge" | "skills" | "browser" | "diagnostics";

export type RightPanel = "none" | "activity" | "diff" | "changes" | "file";

export interface DiffTarget {
  /** Workspace-relative path the `files` tool reported. */
  path: string;
  /** Proposed file content from the tool arguments (`files.write`). */
  content: string;
  /** For `edit`: the anchor and its replacement, applied to the file on disk. */
  oldString?: string;
  newString?: string;
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
  /** Workspace the active conversation is bound to; drives the browser pane so
   * a manual sign-in lands in the same profile the agent uses. */
  workspaceId: string | null;
  setWorkspaceId: (id: string | null) => void;
  runId: string | null;
  setRunId: (id: string | null) => void;
  modelId: string;
  setModelId: (model: string) => void;
  mode: RunMode;
  setMode: (mode: RunMode) => void;
  /** Top-level UI surface (chat vs. agent). Persisted to localStorage. */
  surface: Surface;
  setSurface: (surface: Surface) => void;
  /** Sidebar thread list filter. Persisted to localStorage. */
  threadFilter: ThreadFilter;
  setThreadFilter: (filter: ThreadFilter) => void;
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
  /** The keyboard-shortcut cheat sheet. Session-only (not persisted): it is a
   *  reference the user opens on demand, not a remembered panel. */
  shortcutsOpen: boolean;
  setShortcutsOpen: (open: boolean) => void;
  /** Force the first-run wizard open for this session.
   *
   * Deliberately *not* persisted. Whether the wizard shows on its own is a
   * question about the installation (has anything been set up yet?), and that
   * answer lives in the backend; this flag only covers "the user just asked to
   * see it again from Settings". Writing it to localStorage would make
   * dismissing the wizard permanent in a way Settings cannot undo.
   */
  onboardingForcedOpen: boolean;
  setOnboardingForcedOpen: (open: boolean) => void;
}

const SIDEBAR_KEY = "mix-agent-sidebar-open";
const RIGHT_PANEL_KEY = "mix-agent-right-panel";
const FILE_TREE_KEY = "mix-agent-file-tree-open";
const ACTIVE_PROJECT_KEY = "mix-agent-active-project";
const SETTINGS_TAB_KEY = "mix-agent-settings-tab";
const SURFACE_KEY = "mix-agent-surface";
const THREAD_FILTER_KEY = "mix-agent-thread-filter";

const SURFACES: Surface[] = ["chat", "desktop-agent"];
const THREAD_FILTERS: ThreadFilter[] = ["all", "chat", "desktop-agent"];

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

function readSurface(): Surface {
  try {
    const stored = localStorage.getItem(SURFACE_KEY);
    if (stored && SURFACES.includes(stored as Surface)) return stored as Surface;
  } catch {
    // fall through
  }
  return "chat";
}

function readThreadFilter(): ThreadFilter {
  try {
    const stored = localStorage.getItem(THREAD_FILTER_KEY);
    if (stored && THREAD_FILTERS.includes(stored as ThreadFilter)) return stored as ThreadFilter;
  } catch {
    // fall through
  }
  return "all";
}

/**
 * Pick a default `mode` for a surface that the surface actually supports.
 * Used by `setSurface` to keep `mode` consistent across surfaces.
 */
export function defaultModeForSurface(surface: Surface): RunMode {
  return SURFACE_MODES[surface][0];
}

/**
 * Which surface a conversation belongs to, derived from its run mode.
 *
 * `ConversationSummary` carries a `mode`, not a surface, so the sidebar's
 * thread filter has to map one back to the other. It is the inverse of
 * `SURFACE_MODES`: `agent` runs live on the desktop-agent surface, everything
 * else (`chat`, `thinking`) is the chat surface.
 */
export function surfaceForMode(mode: string): Surface {
  return mode === "agent" ? "desktop-agent" : "chat";
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
  workspaceId: null,
  setWorkspaceId: (workspaceId) => set({ workspaceId }),
  runId: null,
  setRunId: (id) => set({ runId: id }),
  modelId: "",
  setModelId: (model) => set({ modelId: model }),
  mode: "chat",
  setMode: (mode) => {
    // Refuse modes that no active surface supports and snap to the surface
    // default instead. This keeps `mode` always within `SURFACE_MODES[surface]`
    // so the Composer never offers an unsupported choice.
    set((state) => {
      if (SURFACE_MODES[state.surface].includes(mode)) return { mode };
      return { mode: SURFACE_MODES[state.surface][0] };
    });
  },
  surface: readSurface(),
  setSurface: (surface) => {
    try {
      localStorage.setItem(SURFACE_KEY, surface);
    } catch {
      // best-effort
    }
    set((state) => {
      // Snap `mode` into the new surface's allowed set so legacy state from
      // a different surface can never leak across (e.g. `agent` on the chat
      // surface, which has no tool budget).
      const nextMode = SURFACE_MODES[surface].includes(state.mode)
        ? state.mode
        : SURFACE_MODES[surface][0];
      return { surface, mode: nextMode };
    });
  },
  threadFilter: readThreadFilter(),
  setThreadFilter: (threadFilter) => {
    try {
      localStorage.setItem(THREAD_FILTER_KEY, threadFilter);
    } catch {
      // best-effort
    }
    set({ threadFilter });
  },
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
  shortcutsOpen: false,
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  onboardingForcedOpen: false,
  setOnboardingForcedOpen: (onboardingForcedOpen) => set({ onboardingForcedOpen }),
}));
