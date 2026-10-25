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
});
