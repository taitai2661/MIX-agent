import { create } from "zustand";
import { getLanguage, setLanguage, type Language } from "./i18n";
import { storedTheme, storeTheme, type Theme } from "./theme";
import {
  getBoolean,
  getString,
  removeKey,
  setBoolean,
  setString,
} from "./storage";

export type View = "chat" | "agents" | "projects" | "schedules" | "settings";

/**
 * Top-level UI surface the user is currently in.
 * - `chat`          — Simple Q&A experience (chat / thinking only, no workspace chrome)
 * - `desktop-agent` — Agentic workspace (agent mode, file tree, changes, activity)
 */
export type Surface = "chat" | "desktop-agent";

/** Run mode understood by the Rust Run Engine (see `core/src/mode.rs`). */
export type RunMode = "chat" | "thinking" | "agent";

/** Canonical union for every run mode the engine understands.
 *
 * `RunMode` (above) is kept as an alias for existing imports. Backend
 * payloads (`ConversationSummary.mode`, …) stay `string`: the engine may
 * send a future mode this build does not know, and `surfaceForMode` maps
 * anything unknown to `chat` instead of throwing. */
export type Mode = RunMode;

/** Filter applied to the sidebar thread list. */
export type ThreadFilter = "all" | Surface;

/** Modes a surface allows the user to pick from in the Composer. */
export const SURFACE_MODES: Record<Surface, RunMode[]> = {
  chat: ["chat", "thinking"],
  "desktop-agent": ["agent"],
};

/** Tabs inside the Settings pane (see features/settings/SettingsPane.tsx). */
export type SettingsTab =
  | "appearance"
  | "general"
  | "network"
  | "updates"
  | "backup"
  | "providers"
  | "models"
  | "tools"
  | "approvals"
  | "mcp"
  | "memory"
  | "knowledge"
  | "skills"
  | "browser"
  | "diagnostics";

export type RightPanel = "none" | "activity" | "diff" | "changes" | "file";

/** Key shortcut used to send a chat message in the Composer. */
export type SendShortcut = "enter" | "modifier";

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
  chatConversationId: string | null;
  setChatConversationId: (id: string | null) => void;
  agentConversationId: string | null;
  setAgentConversationId: (id: string | null) => void;
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
  /** Key shortcut used to send messages (Enter vs Cmd/Ctrl+Enter). */
  sendShortcut: SendShortcut;
  setSendShortcut: (shortcut: SendShortcut) => void;
  /** Whether the right panel is docked (split view) or floating overlay. */
  rightPanelDocked: boolean;
  setRightPanelDocked: (docked: boolean) => void;
  toggleRightPanelDocked: () => void;
  /** Right panel custom or toggled width in pixels. */
  rightPanelWidth: number;
  setRightPanelWidth: (width: number) => void;
  toggleRightPanelWidth: () => void;
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
const SEND_SHORTCUT_KEY = "mix-agent-send-shortcut";
const RIGHT_PANEL_WIDTH_KEY = "mix-agent-right-panel-width";
const RIGHT_PANEL_DOCKED_KEY = "mix-agent-right-panel-docked";
const CHAT_CONVERSATION_KEY = "mix-agent-chat-conversation-id";
const AGENT_CONVERSATION_KEY = "mix-agent-agent-conversation-id";

export const DEFAULT_RIGHT_PANEL_WIDTH = 380;
export const WIDE_RIGHT_PANEL_WIDTH = 560;

const SURFACES: Surface[] = ["chat", "desktop-agent"];
const THREAD_FILTERS: ThreadFilter[] = ["all", "chat", "desktop-agent"];

function readActiveProject(): string | null {
  const stored = getString(ACTIVE_PROJECT_KEY);
  // An empty string is a tombstone of nothing: treat it as unset so callers
  // never try to resolve a project with no id.
  return stored ? stored : null;
}

/**
 * Drop a persisted `activeProjectId` that no longer names a known project
 * (e.g. the project was deleted from another window or device). Call this
 * once the project list has loaded; it clears both the store and the key so
 * a stale id can never resurface on the next boot.
 */
export function pruneStaleActiveProject(knownIds: readonly string[]): void {
  const current = useUi.getState().activeProjectId;
  if (current !== null && !knownIds.includes(current)) {
    useUi.getState().setActiveProjectId(null);
  }
}

function readBoolean(key: string, fallback: boolean): boolean {
  return getBoolean(key, fallback);
}

function readRightPanel(): RightPanel {
  const stored = getString(RIGHT_PANEL_KEY);
  if (
    stored === "none" ||
    stored === "activity" ||
    stored === "diff" ||
    stored === "changes" ||
    stored === "file"
  ) {
    return stored;
  }
  // Codex-like default: the centre column owns the full width until the user
  // explicitly opens a side panel. Defaulting to "activity" squeezed every
  // fresh install into a narrow transcript.
  return "none";
}

function readRightPanelDocked(): boolean {
  return readBoolean(RIGHT_PANEL_DOCKED_KEY, true);
}

const SETTING_TABS: SettingsTab[] = [
  "appearance",
  "general",
  "network",
  "updates",
  "backup",
  "providers",
  "models",
  "tools",
  "approvals",
  "mcp",
  "memory",
  "knowledge",
  "skills",
  "browser",
  "diagnostics",
];

function readSettingsTab(): SettingsTab {
  const stored = getString(SETTINGS_TAB_KEY);
  if (stored && SETTING_TABS.includes(stored as SettingsTab))
    return stored as SettingsTab;
  return "appearance";
}

function readSurface(): Surface {
  const stored = getString(SURFACE_KEY);
  if (stored && SURFACES.includes(stored as Surface)) return stored as Surface;
  return "chat";
}

function readThreadFilter(): ThreadFilter {
  const stored = getString(THREAD_FILTER_KEY);
  if (stored && THREAD_FILTERS.includes(stored as ThreadFilter))
    return stored as ThreadFilter;
  return "all";
}

function readSendShortcut(): SendShortcut {
  const stored = getString(SEND_SHORTCUT_KEY);
  if (stored === "modifier" || stored === "enter") return stored;
  return "enter";
}

function readStoredConversation(key: string): string | null {
  const stored = getString(key);
  return stored ? stored : null;
}

function readRightPanelWidth(): number {
  const stored = getString(RIGHT_PANEL_WIDTH_KEY);
  if (stored) {
    const val = Number(stored);
    if (!Number.isNaN(val) && val >= 320 && val <= 900) return val;
  }
  return DEFAULT_RIGHT_PANEL_WIDTH;
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
export function surfaceForMode(mode: Mode | (string & {})): Surface {
  return mode === "agent" ? "desktop-agent" : "chat";
}

const initialSurface = readSurface();
const initialChatConv = readStoredConversation(CHAT_CONVERSATION_KEY);
const initialAgentConv = readStoredConversation(AGENT_CONVERSATION_KEY);
const initialConv =
  initialSurface === "desktop-agent" ? initialAgentConv : initialChatConv;

export const useUi = create<UiState>((set) => ({
  view: "chat",
  setView: (view) => set({ view }),
  settingsTab: readSettingsTab(),
  setSettingsTab: (settingsTab) => {
    setString(SETTINGS_TAB_KEY, settingsTab);
    set({ settingsTab });
  },
  openSettings: (tab) =>
    set((state) => {
      const settingsTab = tab ?? state.settingsTab;
      // Persist the target tab: without this a reload always reopened the
      // default tab even when the user was deep in another section.
      setString(SETTINGS_TAB_KEY, settingsTab);
      return { view: "settings", settingsTab };
    }),
  conversationId: initialConv,
  chatConversationId: initialChatConv,
  agentConversationId: initialAgentConv,
  setConversationId: (id) =>
    set((state) => {
      if (state.surface === "desktop-agent") {
        if (id) setString(AGENT_CONVERSATION_KEY, id);
        else removeKey(AGENT_CONVERSATION_KEY);
        return { conversationId: id, agentConversationId: id };
      } else {
        if (id) setString(CHAT_CONVERSATION_KEY, id);
        else removeKey(CHAT_CONVERSATION_KEY);
        return { conversationId: id, chatConversationId: id };
      }
    }),
  setChatConversationId: (id) => {
    if (id) setString(CHAT_CONVERSATION_KEY, id);
    else removeKey(CHAT_CONVERSATION_KEY);
    set((state) => ({
      chatConversationId: id,
      conversationId: state.surface === "chat" ? id : state.conversationId,
    }));
  },
  setAgentConversationId: (id) => {
    if (id) setString(AGENT_CONVERSATION_KEY, id);
    else removeKey(AGENT_CONVERSATION_KEY);
    set((state) => ({
      agentConversationId: id,
      conversationId:
        state.surface === "desktop-agent" ? id : state.conversationId,
    }));
  },
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
  surface: initialSurface,
  setSurface: (surface) => {
    setString(SURFACE_KEY, surface);
    set((state) => {
      // Snap `mode` into the new surface's allowed set so legacy state from
      // a different surface can never leak across (e.g. `agent` on the chat
      // surface, which has no tool budget).
      const nextMode = SURFACE_MODES[surface].includes(state.mode)
        ? state.mode
        : SURFACE_MODES[surface][0];
      const targetConvId =
        surface === "desktop-agent"
          ? state.agentConversationId
          : state.chatConversationId;
      return { surface, mode: nextMode, conversationId: targetConvId };
    });
  },
  threadFilter: readThreadFilter(),
  setThreadFilter: (threadFilter) => {
    setString(THREAD_FILTER_KEY, threadFilter);
    set({ threadFilter });
  },
  sidebarOpen: readBoolean(SIDEBAR_KEY, true),
  toggleSidebar: () =>
    set((state) => {
      const sidebarOpen = !state.sidebarOpen;
      setBoolean(SIDEBAR_KEY, sidebarOpen);
      return { sidebarOpen };
    }),
  fileTreeOpen: readBoolean(FILE_TREE_KEY, false),
  toggleFileTree: () =>
    set((state) => {
      const fileTreeOpen = !state.fileTreeOpen;
      setBoolean(FILE_TREE_KEY, fileTreeOpen);
      return { fileTreeOpen };
    }),
  activeProjectId: readActiveProject(),
  setActiveProjectId: (activeProjectId) => {
    if (activeProjectId === null) removeKey(ACTIVE_PROJECT_KEY);
    else setString(ACTIVE_PROJECT_KEY, activeProjectId);
    set({ activeProjectId });
  },
  rightPanel: readRightPanel(),
  setRightPanel: (rightPanel) => {
    setString(RIGHT_PANEL_KEY, rightPanel);
    set({ rightPanel });
  },
  activityRunId: null,
  setActivityRunId: (activityRunId) => set({ activityRunId }),
  diffTarget: null,
  openDiff: (diffTarget) => set({ diffTarget, rightPanel: "diff" }),
  filePreviewPath: null,
  openFilePreview: (filePreviewPath) =>
    set({ filePreviewPath, rightPanel: "file" }),
  gitDiffPath: null,
  setGitDiffPath: (gitDiffPath) => set({ gitDiffPath }),
  sendShortcut: readSendShortcut(),
  setSendShortcut: (sendShortcut) => {
    setString(SEND_SHORTCUT_KEY, sendShortcut);
    set({ sendShortcut });
  },
  rightPanelDocked: readRightPanelDocked(),
  setRightPanelDocked: (rightPanelDocked) => {
    setBoolean(RIGHT_PANEL_DOCKED_KEY, rightPanelDocked);
    set({ rightPanelDocked });
  },
  toggleRightPanelDocked: () =>
    set((state) => {
      const next = !state.rightPanelDocked;
      setBoolean(RIGHT_PANEL_DOCKED_KEY, next);
      return { rightPanelDocked: next };
    }),
  rightPanelWidth: readRightPanelWidth(),
  setRightPanelWidth: (rightPanelWidth) => {
    setString(RIGHT_PANEL_WIDTH_KEY, String(rightPanelWidth));
    set({ rightPanelWidth });
  },
  toggleRightPanelWidth: () =>
    set((state) => {
      const next =
        state.rightPanelWidth >= WIDE_RIGHT_PANEL_WIDTH
          ? DEFAULT_RIGHT_PANEL_WIDTH
          : WIDE_RIGHT_PANEL_WIDTH;
      setString(RIGHT_PANEL_WIDTH_KEY, String(next));
      return { rightPanelWidth: next };
    }),
  locale: getLanguage(),
  setLocale: (locale) => {
    setLanguage(locale);
    set({ locale });
  },
  theme: storedTheme(),
  setTheme: (theme) => {
    storeTheme(theme);
    set({ theme });
  },
  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  shortcutsOpen: false,
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  onboardingForcedOpen: false,
  setOnboardingForcedOpen: (onboardingForcedOpen) =>
    set({ onboardingForcedOpen }),
}));

// Cross-tab sync: another tab changing persisted UI state should reflect
// here instead of going stale until reload. `storage` events only fire in
// tabs that did *not* make the change, so echoing through the setters below
// cannot loop. Each branch re-reads through the validated reader and only
// writes back when the value actually differs.
const SYNCED_KEYS: ReadonlySet<string> = new Set([
  SIDEBAR_KEY,
  RIGHT_PANEL_KEY,
  FILE_TREE_KEY,
  ACTIVE_PROJECT_KEY,
  SETTINGS_TAB_KEY,
  SURFACE_KEY,
  THREAD_FILTER_KEY,
  SEND_SHORTCUT_KEY,
  RIGHT_PANEL_WIDTH_KEY,
  RIGHT_PANEL_DOCKED_KEY,
  CHAT_CONVERSATION_KEY,
  AGENT_CONVERSATION_KEY,
]);

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (!event.key || !SYNCED_KEYS.has(event.key)) return;
    const state = useUi.getState();
    switch (event.key) {
      case CHAT_CONVERSATION_KEY: {
        const next = readStoredConversation(CHAT_CONVERSATION_KEY);
        if (next !== state.chatConversationId)
          state.setChatConversationId(next);
        break;
      }
      case AGENT_CONVERSATION_KEY: {
        const next = readStoredConversation(AGENT_CONVERSATION_KEY);
        if (next !== state.agentConversationId)
          state.setAgentConversationId(next);
        break;
      }
      case SETTINGS_TAB_KEY: {
        const next = readSettingsTab();
        if (next !== state.settingsTab) state.setSettingsTab(next);
        break;
      }
      case SURFACE_KEY: {
        // `setSurface` also snaps `mode` into the surface's allowed set.
        const next = readSurface();
        if (next !== state.surface) state.setSurface(next);
        break;
      }
      case THREAD_FILTER_KEY: {
        const next = readThreadFilter();
        if (next !== state.threadFilter) state.setThreadFilter(next);
        break;
      }
      case SIDEBAR_KEY: {
        const next = readBoolean(SIDEBAR_KEY, true);
        if (next !== state.sidebarOpen) state.toggleSidebar();
        break;
      }
      case FILE_TREE_KEY: {
        const next = readBoolean(FILE_TREE_KEY, false);
        if (next !== state.fileTreeOpen) state.toggleFileTree();
        break;
      }
      case SEND_SHORTCUT_KEY: {
        const next = readSendShortcut();
        if (next !== state.sendShortcut) state.setSendShortcut(next);
        break;
      }
      case RIGHT_PANEL_WIDTH_KEY: {
        const next = readRightPanelWidth();
        if (next !== state.rightPanelWidth) state.setRightPanelWidth(next);
        break;
      }
      case RIGHT_PANEL_DOCKED_KEY: {
        const next = readRightPanelDocked();
        if (next !== state.rightPanelDocked) state.setRightPanelDocked(next);
        break;
      }
      case RIGHT_PANEL_KEY: {
        const next = readRightPanel();
        if (next !== state.rightPanel) state.setRightPanel(next);
        break;
      }
      case ACTIVE_PROJECT_KEY: {
        // `null` covers both "removed" and "emptied" on the other side.
        const next = readActiveProject();
        if (next !== state.activeProjectId) state.setActiveProjectId(next);
        break;
      }
    }
  });
}
