import { create } from "zustand";
import {
  ACCENT_STORAGE_KEY,
  storedAccent,
  storeAccent,
  type Accent,
} from "./accent";
import { getLanguage, setLanguage, type Language } from "./i18n";
import { canFullView } from "./panelTabs";
import { recordUxEvent } from "./uxEvents";
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
export type RunMode = "chat" | "thinking" | "agent" | "plan";

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
  // `plan` is the engine's read-only mode: it may read and search, refuses
  // every mutation, and proposes the change with `user_plan`. It lives on the
  // agent surface because it needs a workspace to investigate.
  "desktop-agent": ["agent", "plan"],
};

/** Tabs inside the Settings pane (see features/settings/SettingsPane.tsx). */
export type SettingsTab =
  | "appearance"
  | "general"
  | "updates"
  | "backup"
  | "spotlight"
  | "providers"
  | "models"
  | "tools"
  | "mcp"
  | "memory"
  | "knowledge"
  | "skills"
  | "browser"
  | "diagnostics";

export type RightPanel =
  | "none"
  | "activity"
  | "usage"
  | "diff"
  | "changes"
  | "file"
  | "browser";

/** How the thread side panel is presented. Codex calls these split/full view. */
export type SidePanelMode = "split" | "full";

/** Who opened the browser pane. Agent-opened panes must never steal focus. */
export type BrowserOpenedBy = "agent" | "user";

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
  /** True while the user is composing a thread that has no row yet.
   *
   * A new chat deliberately keeps `conversationId` null and only creates the
   * backend conversation on first send. This flag distinguishes that blank
   * thread from "nothing selected yet" (boot, deleted thread), which otherwise
   * makes the chat pane adopt the newest existing thread. */
  newThreadDraft: boolean;
  /** Open a blank thread. The conversation row is created on first send. */
  startNewThread: () => void;
  /** Per-surface draft flags. Switching workspaces must not drop a blank
   *  "new thread" on the other side — the composer draft is saved separately,
   *  but `newThreadDraft` decides whether that draft is shown at all. */
  chatNewThreadDraft: boolean;
  agentNewThreadDraft: boolean;
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
  /** Active run mode. Kept in step with the per-surface memory below. */
  mode: RunMode;
  setMode: (mode: RunMode) => void;
  /** Last mode picked on each surface. Switching workspaces restores this
   *  instead of snapping to the surface default: a user who was planning on
   *  the coding surface should still be planning when they come back. */
  chatMode: RunMode;
  agentMode: RunMode;
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
  /** Split (chat + panel) or full view (panel takes the centre column). */
  sidePanelMode: SidePanelMode;
  setSidePanelMode: (mode: SidePanelMode) => void;
  toggleSidePanelMode: () => void;
  /** Open the browser pane and remember whether the agent or the user did it.
   *
   * `options.ratio` is the agent's presentation hint: a 0.2–0.85 share of the
   * shell width. A user-opened pane keeps the remembered split; an agent-opened
   * pane defaults to 50% so a first browser run is readable. */
  openBrowserPanel: (
    by: BrowserOpenedBy,
    options?: { ratio?: number | null },
  ) => void;
  /** Close the browser pane and remember the run it was dismissed in. */
  dismissBrowserPanel: () => void;
  /**
   * Close whichever panel is open. The browser goes through
   * `dismissBrowserPanel` so an outside click or Escape records the dismissal
   * exactly like the panel's own close button does.
   */
  closeRightPanel: () => void;
  browserOpenedBy: BrowserOpenedBy | null;
  /** Run whose browser auto-open the user dismissed (null = none). */
  browserDismissedRunId: string | null;
  /** Auto-open the browser pane when the agent first uses the browser tool. */
  browserAutoOpen: boolean;
  setBrowserAutoOpen: (value: boolean) => void;
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
  /** Share of the shell width used by the browser pane (0.2–0.85).
   *
   * `null` means "use `rightPanelWidth`". The agent sets this when it asks for
   * a specific split; a user drag writes the ratio back so the split survives
   * window resizes. Non-browser panels keep using pixels. */
  rightPanelRatio: number | null;
  setRightPanelRatio: (ratio: number | null) => void;
  locale: Language;
  setLocale: (locale: Language) => void;
  theme: Theme;
  setTheme: (theme: Theme) => void;
  /** Accent hue for focus, selection and primary actions. Persisted locally. */
  accent: Accent;
  setAccent: (accent: Accent) => void;
  /** WebView zoom factor (0.8–1.5). Applied to `documentElement`. */
  zoom: number;
  setZoom: (zoom: number) => void;
  /** Wrap long code lines instead of scrolling horizontally. */
  codeWrap: boolean;
  setCodeWrap: (value: boolean) => void;
  /** User override for the OS reduced-motion preference. */
  reduceMotion: boolean;
  setReduceMotion: (value: boolean) => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  /** Global approval inbox. Session-only, like the palette: it is opened to
   *  answer a blocked run, not a panel the user expects to persist. */
  approvalsOpen: boolean;
  setApprovalsOpen: (open: boolean) => void;
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
const SIDE_PANEL_MODE_KEY = "mix-agent-side-panel-mode";
const BROWSER_AUTO_OPEN_KEY = "mix-agent-browser-auto-open";
const FILE_TREE_KEY = "mix-agent-file-tree-open";
const ACTIVE_PROJECT_KEY = "mix-agent-active-project";
const SETTINGS_TAB_KEY = "mix-agent-settings-tab";
const SURFACE_KEY = "mix-agent-surface";
const THREAD_FILTER_KEY = "mix-agent-thread-filter";
const SEND_SHORTCUT_KEY = "mix-agent-send-shortcut";
const RIGHT_PANEL_WIDTH_KEY = "mix-agent-right-panel-width";
const RIGHT_PANEL_RATIO_KEY = "mix-agent-right-panel-ratio";
const RIGHT_PANEL_DOCKED_KEY = "mix-agent-right-panel-docked";
const CHAT_CONVERSATION_KEY = "mix-agent-chat-conversation-id";
const AGENT_CONVERSATION_KEY = "mix-agent-agent-conversation-id";
const CHAT_MODE_KEY = "mix-agent-chat-mode";
const AGENT_MODE_KEY = "mix-agent-agent-mode";
const ZOOM_KEY = "mix-agent-zoom";
const CODE_WRAP_KEY = "mix-agent-code-wrap";
const REDUCE_MOTION_KEY = "mix-agent-reduce-motion";

export const MIN_ZOOM = 0.8;
export const MAX_ZOOM = 1.5;

export const DEFAULT_RIGHT_PANEL_WIDTH = 380;
export const WIDE_RIGHT_PANEL_WIDTH = 560;
export const MIN_RIGHT_PANEL_WIDTH = 320;
export const MAX_RIGHT_PANEL_WIDTH = 1600;
export const DEFAULT_BROWSER_PANEL_RATIO = 0.5;
export const MIN_BROWSER_PANEL_RATIO = 0.2;
export const MAX_BROWSER_PANEL_RATIO = 0.85;

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
    stored === "usage" ||
    stored === "diff" ||
    stored === "changes" ||
    stored === "file" ||
    stored === "browser"
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

function readSidePanelMode(): SidePanelMode {
  return getString(SIDE_PANEL_MODE_KEY) === "full" ? "full" : "split";
}

const SETTING_TABS: SettingsTab[] = [
  "appearance",
  "general",
  "updates",
  "backup",
  "spotlight",
  "providers",
  "models",
  "tools",
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

export function clampZoom(value: number): number {
  if (!Number.isFinite(value)) return 1;
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(value * 10) / 10));
}

function readZoom(): number {
  const stored = Number(getString(ZOOM_KEY));
  return Number.isFinite(stored) && stored > 0 ? clampZoom(stored) : 1;
}

/** Push the display preferences onto the document root.
 *
 * `zoom` is a Chromium/WebKit proprietary property but Tauri only ever runs
 * on those engines (WKWebView / WebView2 / WebKitGTK), so it is the cheapest
 * layout-zoom path. Firefox ignores it, so we also set a standard
 * `font-size` fallback on the root: `rem`-based layouts still scale. */
export function applyDisplayPrefs(prefs: {
  zoom: number;
  codeWrap: boolean;
  reduceMotion: boolean;
}): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const zoom = clampZoom(prefs.zoom);
  // `zoom` in `CSSStyleDeclaration` is non-standard: lib.dom omits it, so go
  // through `setProperty` to stay type-safe.
  (root.style as CSSStyleDeclaration).setProperty("zoom", String(zoom));
  // Standard fallback for engines without `zoom` (e.g. Firefox dev preview).
  root.style.fontSize = `${16 * zoom}px`;
  root.dataset.codeWrap = prefs.codeWrap ? "wrap" : "scroll";
  root.dataset.reduceMotion = prefs.reduceMotion ? "true" : "false";
}

function readStoredConversation(key: string): string | null {
  const stored = getString(key);
  return stored ? stored : null;
}

function readRightPanelWidth(): number {
  return readPanelWidthValue(getString(RIGHT_PANEL_WIDTH_KEY));
}

function readPanelWidthValue(stored: string | null): number {
  if (stored) {
    const val = Number(stored);
    if (
      !Number.isNaN(val) &&
      val >= MIN_RIGHT_PANEL_WIDTH &&
      val <= MAX_RIGHT_PANEL_WIDTH
    ) {
      return val;
    }
  }
  return DEFAULT_RIGHT_PANEL_WIDTH;
}

/** Clamp a split ratio into the range the panel can render. */
export function clampBrowserPanelRatio(value: number): number {
  return Math.min(
    MAX_BROWSER_PANEL_RATIO,
    Math.max(MIN_BROWSER_PANEL_RATIO, value),
  );
}

/** Codex remembers the side-panel width per workspace; so do we. */
function workspaceWidthKey(workspaceId: string): string {
  return `${RIGHT_PANEL_WIDTH_KEY}:${workspaceId}`;
}

/** The split ratio is remembered per workspace as well. */
function workspaceRatioKey(workspaceId: string): string {
  return `${RIGHT_PANEL_RATIO_KEY}:${workspaceId}`;
}

function readRightPanelRatioValue(stored: string | null): number | null {
  if (!stored) return null;
  const value = Number(stored);
  if (!Number.isFinite(value)) return null;
  if (value < MIN_BROWSER_PANEL_RATIO || value > MAX_BROWSER_PANEL_RATIO) {
    return null;
  }
  return value;
}

function readRightPanelRatio(): number | null {
  return readRightPanelRatioValue(getString(RIGHT_PANEL_RATIO_KEY));
}

/**
 * Pick a default `mode` for a surface that the surface actually supports.
 * Used by `setSurface` to keep `mode` consistent across surfaces.
 */
export function defaultModeForSurface(surface: Surface): RunMode {
  return SURFACE_MODES[surface][0];
}

/** Read the last mode used on a surface, falling back to its default. Stored
 *  values are validated against `SURFACE_MODES` so a mode removed in a later
 *  build can never leak across the workspace split. */
function readModeForSurface(surface: Surface): RunMode {
  const key = surface === "chat" ? CHAT_MODE_KEY : AGENT_MODE_KEY;
  const stored = getString(key);
  if (stored && SURFACE_MODES[surface].includes(stored as RunMode)) {
    return stored as RunMode;
  }
  return defaultModeForSurface(surface);
}

/**
 * Which surface a conversation belongs to, derived from its run mode.
 *
 * `ConversationSummary` carries a `mode`, not a surface, so the sidebar's
 * thread filter has to map one back to the other. It is the inverse of
 * `SURFACE_MODES`: `agent` and `plan` runs live on the desktop-agent surface,
 * everything else (`chat`, `thinking`) is the chat surface.
 */
export function surfaceForMode(mode: Mode | (string & {})): Surface {
  return mode === "agent" || mode === "plan" ? "desktop-agent" : "chat";
}

const initialSurface = readSurface();
const initialChatConv = readStoredConversation(CHAT_CONVERSATION_KEY);
const initialAgentConv = readStoredConversation(AGENT_CONVERSATION_KEY);
const initialConv =
  initialSurface === "desktop-agent" ? initialAgentConv : initialChatConv;
const initialChatMode = readModeForSurface("chat");
const initialAgentMode = readModeForSurface("desktop-agent");
const initialMode =
  initialSurface === "desktop-agent" ? initialAgentMode : initialChatMode;
const initialRightPanel = readRightPanel();
// A persisted "full" may belong to a panel that the current surface cannot
// show (e.g. the browser restored on the chat surface). Clamp it on boot so
// the first render does not hide the chat behind a full-view panel.
const initialSidePanelMode: SidePanelMode =
  readSidePanelMode() === "full" &&
  !canFullView(initialRightPanel, initialSurface === "desktop-agent")
    ? "split"
    : readSidePanelMode();

export const useUi = create<UiState>((set, get) => ({
  view: "chat",
  setView: (view) => set({ view }),
  settingsTab: readSettingsTab(),
  setSettingsTab: (settingsTab) => {
    recordUxEvent("settings_opened", { tab: settingsTab });
    setString(SETTINGS_TAB_KEY, settingsTab);
    set({ settingsTab });
  },
  openSettings: (tab) =>
    set((state) => {
      const settingsTab = tab ?? state.settingsTab;
      recordUxEvent("settings_opened", { tab: settingsTab });
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
      // The Activity/Diff/File panels describe one thread. Leaving their
      // targets in place while switching threads showed the previous task's
      // tools and files beside the new one — and a "close" that acted on the
      // wrong conversation.
      const cleared = {
        activityRunId: null,
        diffTarget: null,
        filePreviewPath: null,
      };
      if (state.surface === "desktop-agent") {
        if (id) setString(AGENT_CONVERSATION_KEY, id);
        else removeKey(AGENT_CONVERSATION_KEY);
        return {
          conversationId: id,
          agentConversationId: id,
          agentNewThreadDraft: false,
          newThreadDraft: false,
          ...cleared,
        };
      } else {
        if (id) setString(CHAT_CONVERSATION_KEY, id);
        else removeKey(CHAT_CONVERSATION_KEY);
        return {
          conversationId: id,
          chatConversationId: id,
          chatNewThreadDraft: false,
          newThreadDraft: false,
          ...cleared,
        };
      }
    }),
  newThreadDraft: false,
  chatNewThreadDraft: false,
  agentNewThreadDraft: false,
  startNewThread: () =>
    set((state) => {
      const cleared = {
        activityRunId: null,
        diffTarget: null,
        filePreviewPath: null,
      };
      if (state.surface === "desktop-agent") {
        removeKey(AGENT_CONVERSATION_KEY);
        return {
          conversationId: null,
          agentConversationId: null,
          agentNewThreadDraft: true,
          newThreadDraft: true,
          ...cleared,
        };
      }
      removeKey(CHAT_CONVERSATION_KEY);
      return {
        conversationId: null,
        chatConversationId: null,
        chatNewThreadDraft: true,
        newThreadDraft: true,
        ...cleared,
      };
    }),
  setChatConversationId: (id) => {
    if (id) setString(CHAT_CONVERSATION_KEY, id);
    else removeKey(CHAT_CONVERSATION_KEY);
    set((state) => ({
      chatConversationId: id,
      chatNewThreadDraft: false,
      newThreadDraft: false,
      conversationId: state.surface === "chat" ? id : state.conversationId,
    }));
  },
  setAgentConversationId: (id) => {
    if (id) setString(AGENT_CONVERSATION_KEY, id);
    else removeKey(AGENT_CONVERSATION_KEY);
    set((state) => ({
      agentConversationId: id,
      agentNewThreadDraft: false,
      newThreadDraft: false,
      conversationId:
        state.surface === "desktop-agent" ? id : state.conversationId,
    }));
  },
  workspaceId: null,
  setWorkspaceId: (workspaceId) =>
    set(() => {
      if (!workspaceId) return { workspaceId };
      const storedWidth = getString(workspaceWidthKey(workspaceId));
      const storedRatio = readRightPanelRatioValue(
        getString(workspaceRatioKey(workspaceId)),
      );
      if (storedWidth === null && storedRatio === null) return { workspaceId };
      return {
        workspaceId,
        rightPanelWidth:
          storedWidth === null
            ? get().rightPanelWidth
            : readPanelWidthValue(storedWidth),
        rightPanelRatio: storedRatio,
      };
    }),
  runId: null,
  setRunId: (id) => set({ runId: id }),
  modelId: "",
  setModelId: (model) => set({ modelId: model }),
  mode: initialMode,
  chatMode: initialChatMode,
  agentMode: initialAgentMode,
  setMode: (mode) => {
    // Refuse modes that no active surface supports and snap to the surface
    // default instead. This keeps `mode` always within `SURFACE_MODES[surface]`
    // so the Composer never offers an unsupported choice.
    const surface = get().surface;
    if (!SURFACE_MODES[surface].includes(mode)) {
      set({ mode: defaultModeForSurface(surface) });
      return;
    }
    // Remember the pick per surface so switching workspaces is not a reset.
    setString(surface === "chat" ? CHAT_MODE_KEY : AGENT_MODE_KEY, mode);
    set(
      surface === "chat" ? { mode, chatMode: mode } : { mode, agentMode: mode },
    );
  },
  surface: initialSurface,
  setSurface: (surface) => {
    setString(SURFACE_KEY, surface);
    // Full view is a transient expansion of the current surface; carrying it
    // across a surface switch could hide the new surface's chat behind a
    // panel that merely survived the transition.
    setString(SIDE_PANEL_MODE_KEY, "split");
    set((state) => {
      // Restore the mode last used on the target surface. Persisted state from
      // an older build (or a mode removed in an upgrade) may still be invalid,
      // so validate before applying and fall back to the surface default.
      const remembered =
        surface === "desktop-agent" ? state.agentMode : state.chatMode;
      const nextMode = SURFACE_MODES[surface].includes(remembered)
        ? remembered
        : defaultModeForSurface(surface);
      const targetConvId =
        surface === "desktop-agent"
          ? state.agentConversationId
          : state.chatConversationId;
      const draft =
        surface === "desktop-agent"
          ? state.agentNewThreadDraft
          : state.chatNewThreadDraft;
      return {
        surface,
        mode: nextMode,
        // Heal a stale remembered value so the next switch validates cleanly.
        ...(surface === "chat"
          ? { chatMode: nextMode }
          : { agentMode: nextMode }),
        conversationId: draft ? null : targetConvId,
        newThreadDraft: draft,
        view: "chat",
        activityRunId: null,
        diffTarget: null,
        filePreviewPath: null,
        sidePanelMode: "split",
      };
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
  rightPanel: initialRightPanel,
  setRightPanel: (rightPanel) => {
    // Full view only makes sense while a pane that supports it is visible:
    // closing the panel or opening an agent-only target (diff/file) returns
    // to the split layout instead of leaving a hidden mode behind.
    const keepFull = canFullView(rightPanel, get().surface === "desktop-agent");
    setString(RIGHT_PANEL_KEY, rightPanel);
    if (!keepFull) setString(SIDE_PANEL_MODE_KEY, "split");
    set((state) => ({
      rightPanel,
      sidePanelMode: keepFull ? state.sidePanelMode : "split",
    }));
  },
  sidePanelMode: initialSidePanelMode,
  setSidePanelMode: (sidePanelMode) => {
    setString(SIDE_PANEL_MODE_KEY, sidePanelMode);
    set({ sidePanelMode });
  },
  toggleSidePanelMode: () =>
    set((state) => {
      const sidePanelMode = state.sidePanelMode === "full" ? "split" : "full";
      setString(SIDE_PANEL_MODE_KEY, sidePanelMode);
      return { sidePanelMode };
    }),
  browserOpenedBy: null,
  browserDismissedRunId: null,
  openBrowserPanel: (browserOpenedBy, options = {}) => {
    setString(RIGHT_PANEL_KEY, "browser");
    // Resolve the requested ratio before `set` so the same render that opens
    // the pane already has the right width. An agent-open without a hint gets
    // the Codex-like 50% default; a user-open keeps the remembered split.
    let rightPanelRatio = get().rightPanelRatio;
    const requested = options.ratio;
    if (typeof requested === "number" && Number.isFinite(requested)) {
      rightPanelRatio = clampBrowserPanelRatio(requested);
      setString(RIGHT_PANEL_RATIO_KEY, String(rightPanelRatio));
      const workspaceId = get().workspaceId;
      if (workspaceId) {
        setString(workspaceRatioKey(workspaceId), String(rightPanelRatio));
      }
    } else if (browserOpenedBy === "agent" && rightPanelRatio === null) {
      rightPanelRatio = DEFAULT_BROWSER_PANEL_RATIO;
    }
    set((state) => ({
      rightPanel: "browser",
      browserOpenedBy,
      browserDismissedRunId: null,
      rightPanelRatio,
      // Full view is a transient expansion of an already-open pane. Opening
      // from closed (especially an agent auto-open) starts in split view so
      // the chat does not disappear.
      sidePanelMode:
        state.rightPanel === "none" ? "split" : state.sidePanelMode,
    }));
  },
  dismissBrowserPanel: () => {
    setString(RIGHT_PANEL_KEY, "none");
    setString(SIDE_PANEL_MODE_KEY, "split");
    set((state) => ({
      rightPanel: "none",
      sidePanelMode: "split",
      // A manual close is respected for the rest of this run. The next run has
      // a different id and may auto-open again; "manual" means no run was
      // active, so a later run is free to auto-open.
      browserDismissedRunId: state.runId ?? "manual",
    }));
  },
  closeRightPanel: () => {
    const state = get();
    if (state.rightPanel === "browser") state.dismissBrowserPanel();
    else state.setRightPanel("none");
  },
  browserAutoOpen: readBoolean(BROWSER_AUTO_OPEN_KEY, true),
  setBrowserAutoOpen: (browserAutoOpen) => {
    setBoolean(BROWSER_AUTO_OPEN_KEY, browserAutoOpen);
    set({ browserAutoOpen });
  },
  activityRunId: null,
  setActivityRunId: (activityRunId) => set({ activityRunId }),
  diffTarget: null,
  openDiff: (diffTarget) => {
    setString(SIDE_PANEL_MODE_KEY, "split");
    set({ diffTarget, rightPanel: "diff", sidePanelMode: "split" });
  },
  filePreviewPath: null,
  openFilePreview: (filePreviewPath) => {
    setString(SIDE_PANEL_MODE_KEY, "split");
    set({ filePreviewPath, rightPanel: "file", sidePanelMode: "split" });
  },
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
    const workspaceId = get().workspaceId;
    if (workspaceId) {
      setString(workspaceWidthKey(workspaceId), String(rightPanelWidth));
    }
    set({ rightPanelWidth });
  },
  rightPanelRatio: readRightPanelRatio(),
  setRightPanelRatio: (rightPanelRatio) => {
    if (rightPanelRatio === null) {
      removeKey(RIGHT_PANEL_RATIO_KEY);
    } else {
      const ratio = clampBrowserPanelRatio(rightPanelRatio);
      setString(RIGHT_PANEL_RATIO_KEY, String(ratio));
      const workspaceId = get().workspaceId;
      if (workspaceId) {
        setString(workspaceRatioKey(workspaceId), String(ratio));
      }
      set({ rightPanelRatio: ratio });
      return;
    }
    const workspaceId = get().workspaceId;
    if (workspaceId) removeKey(workspaceRatioKey(workspaceId));
    set({ rightPanelRatio: null });
  },
  toggleRightPanelWidth: () => {
    const state = get();
    state.setRightPanelWidth(
      state.rightPanelWidth >= WIDE_RIGHT_PANEL_WIDTH
        ? DEFAULT_RIGHT_PANEL_WIDTH
        : WIDE_RIGHT_PANEL_WIDTH,
    );
  },
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
  accent: storedAccent(),
  setAccent: (accent) => {
    storeAccent(accent);
    set({ accent });
  },
  zoom: readZoom(),
  setZoom: (zoom) => {
    const value = clampZoom(zoom);
    setString(ZOOM_KEY, String(value));
    set({ zoom: value });
    applyDisplayPrefs({ ...get(), zoom: value });
  },
  codeWrap: readBoolean(CODE_WRAP_KEY, false),
  setCodeWrap: (codeWrap) => {
    setBoolean(CODE_WRAP_KEY, codeWrap);
    set({ codeWrap });
    applyDisplayPrefs({ ...get(), codeWrap });
  },
  reduceMotion: readBoolean(REDUCE_MOTION_KEY, false),
  setReduceMotion: (reduceMotion) => {
    setBoolean(REDUCE_MOTION_KEY, reduceMotion);
    set({ reduceMotion });
    applyDisplayPrefs({ ...get(), reduceMotion });
  },
  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => {
    if (paletteOpen) recordUxEvent("search_opened", { location: "palette" });
    set({ paletteOpen });
  },
  approvalsOpen: false,
  setApprovalsOpen: (approvalsOpen) => set({ approvalsOpen }),
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
  RIGHT_PANEL_RATIO_KEY,
  RIGHT_PANEL_DOCKED_KEY,
  CHAT_CONVERSATION_KEY,
  AGENT_CONVERSATION_KEY,
  ZOOM_KEY,
  CODE_WRAP_KEY,
  REDUCE_MOTION_KEY,
  ACCENT_STORAGE_KEY,
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
      case RIGHT_PANEL_RATIO_KEY: {
        const next = readRightPanelRatio();
        if (next !== state.rightPanelRatio) state.setRightPanelRatio(next);
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
      case SIDE_PANEL_MODE_KEY: {
        const next = readSidePanelMode();
        if (next !== state.sidePanelMode) state.setSidePanelMode(next);
        break;
      }
      case BROWSER_AUTO_OPEN_KEY: {
        const next = readBoolean(BROWSER_AUTO_OPEN_KEY, true);
        if (next !== state.browserAutoOpen) state.setBrowserAutoOpen(next);
        break;
      }
      case ACTIVE_PROJECT_KEY: {
        // `null` covers both "removed" and "emptied" on the other side.
        const next = readActiveProject();
        if (next !== state.activeProjectId) state.setActiveProjectId(next);
        break;
      }
      case ACCENT_STORAGE_KEY: {
        const next = storedAccent();
        if (next !== state.accent) state.setAccent(next);
        break;
      }
      case ZOOM_KEY: {
        const next = readZoom();
        if (next !== state.zoom) state.setZoom(next);
        break;
      }
      case CODE_WRAP_KEY: {
        const next = readBoolean(CODE_WRAP_KEY, false);
        if (next !== state.codeWrap) state.setCodeWrap(next);
        break;
      }
      case REDUCE_MOTION_KEY: {
        const next = readBoolean(REDUCE_MOTION_KEY, false);
        if (next !== state.reduceMotion) state.setReduceMotion(next);
        break;
      }
    }
  });
}

// Apply once at boot, before the first interactive frame. `main.tsx` imports
// the store through App, so this runs before the shell paints.
applyDisplayPrefs(useUi.getState());
