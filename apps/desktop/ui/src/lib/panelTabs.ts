import type { RightPanel } from "./ui";

/** Panels that participate in the right-panel tab strip. */
export type RightPanelTab = "activity" | "usage" | "changes" | "browser";

export interface RightPanelTabDefinition {
  value: RightPanelTab;
  /** i18n key and default label. */
  label: string;
  /** Only available on the agent surface. */
  agentOnly?: boolean;
}

/**
 * Single source of truth for the tab strip.
 *
 * `App` (full-view eligibility), `RightPanel` (header visibility) and
 * `RightPanelHeader` (the strip itself) all derive from this list, so adding a
 * tab cannot leave one of them behind.
 */
export const RIGHT_PANEL_TABS: readonly RightPanelTabDefinition[] = [
  { value: "activity", label: "アクティビティ" },
  { value: "usage", label: "使用状況" },
  { value: "changes", label: "変更", agentOnly: true },
  { value: "browser", label: "ブラウザ", agentOnly: true },
];

const TAB_VALUES = new Set<string>(RIGHT_PANEL_TABS.map((tab) => tab.value));

export function isRightPanelTab(panel: string): panel is RightPanelTab {
  return TAB_VALUES.has(panel);
}

/**
 * Whether a panel can be rendered on the current surface at all.
 *
 * `diff` / `file` are not tabs: they are agent-only targets opened from the
 * transcript or the file tree.
 */
export function isPanelAvailable(
  panel: RightPanel,
  isAgentSurface: boolean,
): boolean {
  if (panel === "none") return false;
  const tab = RIGHT_PANEL_TABS.find((entry) => entry.value === panel);
  if (!tab) return isAgentSurface;
  return !tab.agentOnly || isAgentSurface;
}

/** Panels that may morph into the centre column. */
export function canFullView(
  panel: RightPanel,
  isAgentSurface: boolean,
): boolean {
  return isRightPanelTab(panel) && isPanelAvailable(panel, isAgentSurface);
}
