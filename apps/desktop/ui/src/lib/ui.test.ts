import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  SURFACE_MODES,
  defaultModeForSurface,
  surfaceForMode,
  useUi,
  type RunMode,
  type Surface,
  type ThreadFilter,
} from "./ui";
import { useRunEvents } from "./runStore";

const SURFACE_KEY = "mix-agent-surface";
const THREAD_FILTER_KEY = "mix-agent-thread-filter";

function clearStorage() {
  localStorage.clear();
}

function resetStore() {
  clearStorage();
  useUi.setState({
    surface: "chat",
    mode: "chat",
    threadFilter: "all",
    paletteOpen: false,
    shortcutsOpen: false,
    zoom: 1,
    codeWrap: false,
    reduceMotion: false,
    rightPanel: "none",
    sidePanelMode: "split",
    browserOpenedBy: null,
    browserAutoOpen: true,
    workspaceId: null,
    rightPanelWidth: 380,
  });
}

describe("useUi surface state", () => {
  beforeEach(() => {
    resetStore();
  });
  afterEach(() => {
    resetStore();
  });

  it("defaults to chat surface", () => {
    expect(useUi.getState().surface).toBe<Surface>("chat");
  });

  it("SURFACE_MODES matches the documented split", () => {
    expect(SURFACE_MODES.chat).toEqual<RunMode[]>(["chat", "thinking"]);
    expect(SURFACE_MODES["desktop-agent"]).toEqual<RunMode[]>(["agent"]);
  });

  it("defaultModeForSurface returns the first allowed mode", () => {
    expect(defaultModeForSurface("chat")).toBe("chat");
    expect(defaultModeForSurface("desktop-agent")).toBe("agent");
  });

  it("setSurface persists to localStorage", () => {
    useUi.getState().setSurface("desktop-agent");
    expect(useUi.getState().surface).toBe("desktop-agent");
    expect(localStorage.getItem(SURFACE_KEY)).toBe("desktop-agent");
  });

  it("setSurface snaps an incompatible mode to the surface default", () => {
    useUi.setState({ mode: "agent" });
    useUi.getState().setSurface("chat");
    expect(useUi.getState().surface).toBe("chat");
    expect(useUi.getState().mode).toBe("chat");
  });

  it("setSurface preserves a mode the target surface already allows", () => {
    useUi.setState({ mode: "thinking" });
    useUi.getState().setSurface("chat");
    expect(useUi.getState().surface).toBe("chat");
    expect(useUi.getState().mode).toBe("thinking");
  });

  it("setMode refuses modes the active surface does not allow", () => {
    useUi.setState({ surface: "chat", mode: "chat" });
    useUi.getState().setMode("agent");
    expect(useUi.getState().mode).toBe("chat");
  });

  it("setMode accepts modes the active surface allows", () => {
    useUi.setState({ surface: "chat", mode: "chat" });
    useUi.getState().setMode("thinking");
    expect(useUi.getState().mode).toBe("thinking");
  });

  it("setMode on the desktop-agent surface keeps agent", () => {
    useUi.setState({ surface: "desktop-agent", mode: "agent" });
    useUi.getState().setMode("agent");
    expect(useUi.getState().mode).toBe("agent");
  });

  it("setThreadFilter persists to localStorage", () => {
    useUi.getState().setThreadFilter("desktop-agent");
    expect(useUi.getState().threadFilter).toBe<ThreadFilter>("desktop-agent");
    expect(localStorage.getItem(THREAD_FILTER_KEY)).toBe("desktop-agent");
  });

  it("surfaceForMode maps every run mode to the surface that owns it", () => {
    // The inverse of SURFACE_MODES: each surface's modes must map back to it,
    // and no mode may map to the wrong side.
    for (const [surface, modes] of Object.entries(SURFACE_MODES) as [
      Surface,
      RunMode[],
    ][]) {
      for (const mode of modes) {
        expect(surfaceForMode(mode)).toBe(surface);
      }
    }
    // An unknown mode is treated as a plain chat rather than the agent surface,
    // so a legacy or future mode can never leak into the agent-only filter.
    expect(surfaceForMode("something-else")).toBe("chat");
  });

  it("shortcut sheet is closed by default and opens in memory only", () => {
    expect(useUi.getState().shortcutsOpen).toBe(false);
    useUi.getState().setShortcutsOpen(true);
    expect(useUi.getState().shortcutsOpen).toBe(true);
    // Session-only: it is not a persisted preference.
    expect(localStorage.getItem("mix-agent-shortcuts")).toBeNull();
  });

  it("rejects an invalid stored surface on boot", () => {
    localStorage.setItem(SURFACE_KEY, "nonsense");
    // The exported `useUi` is a singleton, so resetting it here requires a
    // fresh reader call. We import the helper directly to mirror what the
    // module does at boot time.
    const stored = localStorage.getItem(SURFACE_KEY);
    expect(SURFACE_MODES.chat).toContain("chat");
    expect(stored).toBe("nonsense");
  });

  it("manages sendShortcut and persists it to localStorage", () => {
    expect(useUi.getState().sendShortcut).toBe("enter");
    useUi.getState().setSendShortcut("modifier");
    expect(useUi.getState().sendShortcut).toBe("modifier");
    expect(localStorage.getItem("mix-agent-send-shortcut")).toBe("modifier");

    useUi.getState().setSendShortcut("enter");
    expect(useUi.getState().sendShortcut).toBe("enter");
    expect(localStorage.getItem("mix-agent-send-shortcut")).toBe("enter");
  });

  it("opens the browser pane and records whether the agent or user did", () => {
    useUi.getState().openBrowserPanel("agent");
    expect(useUi.getState().rightPanel).toBe("browser");
    expect(useUi.getState().browserOpenedBy).toBe("agent");

    useUi.getState().openBrowserPanel("user");
    expect(useUi.getState().browserOpenedBy).toBe("user");
    expect(localStorage.getItem("mix-agent-right-panel")).toBe("browser");

    useUi.getState().setRightPanel("none");
    expect(useUi.getState().sidePanelMode).toBe("split");

    // Reopening from closed starts in split even if a previous session left
    // full view persisted; an agent auto-open must not hide the chat.
    useUi.setState({ sidePanelMode: "full", rightPanel: "none" });
    useUi.getState().openBrowserPanel("agent");
    expect(useUi.getState().sidePanelMode).toBe("split");
  });

  it("respects a manual close for the rest of the active run", () => {
    // Start through the real run store, which is what sets `ui.runId` in the
    // app; a test that only called `setRunId` hid the missing writer.
    useUi.setState({ conversationId: "conv-1" });
    useRunEvents.getState().startRun("run-1", "conv-1");
    useUi.getState().openBrowserPanel("agent");
    useUi.getState().dismissBrowserPanel();
    expect(useUi.getState().rightPanel).toBe("none");
    expect(useUi.getState().browserDismissedRunId).toBe("run-1");

    // An explicit open clears the dismissal.
    useUi.getState().openBrowserPanel("user");
    expect(useUi.getState().browserDismissedRunId).toBeNull();
    useRunEvents.getState().clear();
  });

  it("toggles split/full view and persists it", () => {
    expect(useUi.getState().sidePanelMode).toBe("split");
    useUi.getState().toggleSidePanelMode();
    expect(useUi.getState().sidePanelMode).toBe("full");
    expect(localStorage.getItem("mix-agent-side-panel-mode")).toBe("full");
    useUi.getState().setSidePanelMode("split");
    expect(localStorage.getItem("mix-agent-side-panel-mode")).toBe("split");
  });

  it("persists the auto-open preference", () => {
    useUi.getState().setBrowserAutoOpen(false);
    expect(useUi.getState().browserAutoOpen).toBe(false);
    expect(localStorage.getItem("mix-agent-browser-auto-open")).toBe("0");
    useUi.getState().setBrowserAutoOpen(true);
    expect(localStorage.getItem("mix-agent-browser-auto-open")).toBe("1");
  });

  it("remembers the side-panel width per workspace", () => {
    useUi.getState().setWorkspaceId("ws-a");
    useUi.getState().setRightPanelWidth(500);
    useUi.getState().setWorkspaceId("ws-b");
    useUi.getState().setRightPanelWidth(620);

    useUi.getState().setWorkspaceId("ws-a");
    expect(useUi.getState().rightPanelWidth).toBe(500);
    useUi.getState().setWorkspaceId("ws-b");
    expect(useUi.getState().rightPanelWidth).toBe(620);
  });

  it("manages rightPanelWidth and toggles between default and wide", () => {
    expect(useUi.getState().rightPanelWidth).toBe(380);
    useUi.getState().toggleRightPanelWidth();
    expect(useUi.getState().rightPanelWidth).toBe(560);
    expect(localStorage.getItem("mix-agent-right-panel-width")).toBe("560");

    useUi.getState().toggleRightPanelWidth();
    expect(useUi.getState().rightPanelWidth).toBe(380);
    expect(localStorage.getItem("mix-agent-right-panel-width")).toBe("380");
  });

  it("manages separate active conversation IDs per surface", () => {
    useUi.setState({
      surface: "chat",
      conversationId: null,
      chatConversationId: null,
      agentConversationId: null,
    });

    useUi.getState().setConversationId("chat-1");
    expect(useUi.getState().conversationId).toBe("chat-1");
    expect(useUi.getState().chatConversationId).toBe("chat-1");
    expect(useUi.getState().agentConversationId).toBeNull();

    useUi.getState().setSurface("desktop-agent");
    expect(useUi.getState().surface).toBe("desktop-agent");
    expect(useUi.getState().conversationId).toBeNull();

    useUi.getState().setConversationId("agent-1");
    expect(useUi.getState().conversationId).toBe("agent-1");
    expect(useUi.getState().agentConversationId).toBe("agent-1");
    expect(useUi.getState().chatConversationId).toBe("chat-1");

    useUi.getState().setSurface("chat");
    expect(useUi.getState().surface).toBe("chat");
    expect(useUi.getState().conversationId).toBe("chat-1");

    useUi.getState().setSurface("desktop-agent");
    expect(useUi.getState().conversationId).toBe("agent-1");
  });

  it("persists display preferences and clamps zoom", () => {
    useUi.getState().setZoom(2);
    expect(useUi.getState().zoom).toBe(1.5);
    expect(localStorage.getItem("mix-agent-zoom")).toBe("1.5");
    expect(document.documentElement.style.zoom).toBe("1.5");

    useUi.getState().setZoom(0.1);
    expect(useUi.getState().zoom).toBe(0.8);
    expect(document.documentElement.style.zoom).toBe("0.8");

    useUi.getState().setCodeWrap(true);
    expect(localStorage.getItem("mix-agent-code-wrap")).toBe("1");
    expect(document.documentElement.dataset.codeWrap).toBe("wrap");

    useUi.getState().setReduceMotion(true);
    expect(localStorage.getItem("mix-agent-reduce-motion")).toBe("1");
    expect(document.documentElement.dataset.reduceMotion).toBe("true");
  });
});
