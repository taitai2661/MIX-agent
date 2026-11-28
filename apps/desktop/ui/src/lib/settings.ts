import { useEffect, useRef } from "react";
import {
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { create } from "zustand";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useToasts } from "@/lib/toast";
import { recordUxEvent } from "@/lib/uxEvents";
import type { SettingsView } from "@/lib/types";

export const SETTINGS_QUERY_KEY = ["settings"] as const;

/**
 * The last persisted settings row, serialised.
 *
 * The Settings form edits the React Query cache directly. Keeping the baseline
 * outside the pane is what makes "unsaved" survive a trip to another view:
 * component-local state used to reset on remount and re-baseline the draft as
 * if it had been saved. A module store also lets the sidebar and the
 * beforeunload guard see the same answer as the Settings pane.
 */
interface SettingsBaselineState {
  baseline: string | null;
  setBaseline: (value: string | null) => void;
}

const useSettingsBaseline = create<SettingsBaselineState>((set) => ({
  baseline: null,
  setBaseline: (baseline) => set({ baseline }),
}));

/** Test seam: start a fresh install with no baseline. */
export function resetSettingsBaseline(): void {
  useSettingsBaseline.setState({ baseline: null });
}

/** Whether the shared draft is currently being written.
 *
 * `dirty` alone cannot tell "waiting for the debounce" from "the last write
 * failed", and showing "saving…" forever after a failure was misleading.
 * `saved` is a short-lived acknowledgement so routine autosaves do not need a
 * toast on every toggle. */
export type SettingsSaveStatus = "idle" | "saving" | "saved" | "error";

export const useSettingsSaveStatus = create<{
  status: SettingsSaveStatus;
  setStatus: (status: SettingsSaveStatus) => void;
}>((set) => ({
  status: "idle",
  setStatus: (status) => set({ status }),
}));

/** How long the "Saved" acknowledgement stays in the Settings header. */
const SAVED_STATUS_MS = 2000;
let savedStatusTimer: number | undefined;

function markSettingsSaved(): void {
  useSettingsSaveStatus.getState().setStatus("saved");
  if (savedStatusTimer !== undefined) window.clearTimeout(savedStatusTimer);
  savedStatusTimer = window.setTimeout(() => {
    savedStatusTimer = undefined;
    if (useSettingsSaveStatus.getState().status === "saved") {
      useSettingsSaveStatus.getState().setStatus("idle");
    }
  }, SAVED_STATUS_MS);
}

/** How often pending tool approvals are polled. One value for every query
 * with this key — different intervals fought over the same cache entry. */
export const PENDING_APPROVALS_POLL_MS = 4000;

/** Values rendered while `get_settings` is still in flight.
 *
 * Display only. The settings row is replaced wholesale on save, so writing
 * these to the backend would erase whatever the user had configured.
 */
export const DEFAULT_SETTINGS: SettingsView = {
  default_model_id: null,
  default_mode: null,
  egress_allowlist: [],
  // Local dev servers stay unreachable until the user explicitly opts in.
  browser_allow_local: false,
  language: null,
  theme: null,
  projects_root: null,
  project_doc_max_bytes: null,
  project_doc_fallback_filenames: [],
  // The backend defaults this to `true` too (memory spec §6): a row read
  // before it arrives must not render as "off".
  memory_auto_formation: true,
  // Personal skills apply across projects; the backend default is `true` too.
  discover_user_skills: true,
  // Memory spec §4 「(3) は既定オフ」 — the one setting here whose honest
  // default is `false`. Seeding it `true` would both mis-render the switch and,
  // if the seeded value ever reached a save, turn on a source §4 keeps off.
  memory_link_embedding_neighbors: false,
  // The unofficial integration is off until the user enables it in Providers.
  chatgpt_enabled: false,
  verify_command: null,
  format_command: null,
  sandbox_terminal: false,
  sandbox_terminal_network: false,
  // Backend default: one day, so an approval prompt survives a trip away from
  // the desk. The executor clamps anything outside 1 minute–7 days.
  approval_ttl_minutes: 24 * 60,
  monthly_budget_usd: null,
  // SQLCipher is not wired yet; the only honest value is `false`.
  database_encryption: false,
  onboarding_completed: null,
  // The backend defaults this to `true` too: auto-check is one manifest
  // request and the payload is signature-verified before installation.
  auto_check_updates: true,
  // Downloading is safe to start in the background; applying still waits for
  // the user to quit the app normally.
  auto_download_updates: true,
  local_scan_on_start: true,
  local_stop_on_exit: true,
  // Spawning `ollama serve` for a user who manages it as a service was a
  // surprise; the Providers pane has an explicit Start button.
  local_auto_start: false,
  local_keep_alive: null,
  local_context_window: null,
  local_prefer: false,
  // The measured-speed signal is only trusted after successful turns; a user
  // who wants the fastest answer opts into `speed` explicitly.
  auto_priority: "balanced",
  // One pre-content retry recovers most transient provider failures without
  // turning a Run into an unbounded model hunt.
  auto_fallback_attempts: 1,
  // System-wide search is opt-in; the backend defaults are `false` too.
  spotlight_enabled: false,
  spotlight_index_body: false,
};

/** Shared access to the settings draft: one query, one cache entry, and a
 * `patch` that refuses to seed the cache with defaults before the load
 * finishes (which would make a later Save persist them). */
export function useSettingsForm() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: SETTINGS_QUERY_KEY,
    queryFn: api.getSettings,
    // The form edits the query cache directly. A fresh observer (switching
    // Settings tabs) with the default staleTime 0 refetched on mount and threw
    // those edits away without warning, leaving the Save flow unusable unless
    // the user never left the tab. Invalidation after a save still refetches.
    staleTime: Infinity,
  });
  const loaded = query.data !== undefined;
  const data = query.data ?? DEFAULT_SETTINGS;
  const baseline = useSettingsBaseline((state) => state.baseline);
  const setBaseline = useSettingsBaseline((state) => state.setBaseline);
  // Baseline once per app run, when the first read lands. Later mounts must
  // not re-baseline: that is exactly what hid unsaved edits after leaving the
  // Settings view.
  useEffect(() => {
    if (!loaded || baseline !== null) return;
    setBaseline(JSON.stringify(query.data));
  }, [baseline, loaded, query.data, setBaseline]);
  const dirty = baseline !== null && JSON.stringify(data) !== baseline;
  const patch = (next: Partial<SettingsView>) => {
    if (!loaded) return;
    queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, {
      ...data,
      ...next,
    });
  };
  // Settled, not loaded: a failed `get_settings` leaves `loaded` false forever,
  // and a caller gating on it would hang rather than fall back to defaults.
  return {
    data,
    loaded,
    settled: !query.isPending,
    dirty,
    baseline,
    setBaseline,
    patch,
    query,
  };
}

/**
 * One write queue for every settings mutation, whether it came from the
 * debounced autosave or a control that saves on click (`useApplySettings`).
 *
 * Running them in parallel let an older full-row snapshot land after a newer
 * one and revert it. Serialising also means `useApplySettings` reads the
 * persisted row after any queued autosave has completed, so it cannot merge
 * onto a row that is about to change underneath it.
 */
let settingsWriteQueue: Promise<void> = Promise.resolve();

function enqueueSettingsWrite<T>(task: () => Promise<T>): Promise<T> {
  const next = settingsWriteQueue.then(task);
  settingsWriteQueue = next.then(
    () => undefined,
    () => undefined,
  );
  return next;
}

/**
 * Keys whose change is consequential enough to offer an Undo toast. Routine
 * toggles (update checks, local runtime preferences) save silently and only
 * light the "Saved" badge: toasting every switch was noise.
 */
const UNDOABLE_SETTINGS_KEYS: ReadonlySet<string> = new Set([
  "projects_root",
  "default_model_id",
  "default_mode",
  "egress_allowlist",
  "verify_command",
  "format_command",
  "sandbox_terminal",
  "sandbox_terminal_network",
  "approval_ttl_minutes",
  "monthly_budget_usd",
  "database_encryption",
  "memory_auto_formation",
  "memory_link_embedding_neighbors",
  "discover_user_skills",
  "spotlight_enabled",
  "spotlight_index_body",
  "local_auto_start",
  "local_keep_alive",
  "local_context_window",
  "local_prefer",
  "onboarding_completed",
]);

function settingsValueEqual(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * The fields that differ between two settings rows.
 *
 * Autosave writes only this patch, merged onto the latest persisted row. The
 * old full-row write let one window overwrite a field another window had just
 * changed; a patch at least scopes each write to what this window edited.
 */
export function diffSettings(
  before: SettingsView,
  after: SettingsView,
): Partial<SettingsView> {
  const patch: Record<string, unknown> = {};
  const beforeRecord = before as unknown as Record<string, unknown>;
  const afterRecord = after as unknown as Record<string, unknown>;
  for (const key of Object.keys(afterRecord)) {
    if (!settingsValueEqual(beforeRecord[key], afterRecord[key])) {
      patch[key] = afterRecord[key];
    }
  }
  return patch as Partial<SettingsView>;
}

/** Whether a patch touches a field that deserves an Undo affordance. */
export function hasUndoableSettingsChange(
  patch: Partial<SettingsView>,
): boolean {
  return Object.keys(patch).some((key) => UNDOABLE_SETTINGS_KEYS.has(key));
}

/**
 * After a successful write, bring the draft back in line with what the server
 * actually stored. The backend may normalise a value (clamp a number, fill a
 * default); leaving the draft untouched would keep `dirty` true forever and
 * re-save the same row on every debounce tick.
 *
 * If the user edited while the write was in flight the cache holds a different
 * object. Keep those newer fields on top of the stored row so the next autosave
 * persists them instead of silently discarding the keystrokes.
 */
function reconcileSettingsDraft(
  queryClient: QueryClient,
  snapshot: SettingsView,
  stored: SettingsView,
): void {
  const current = queryClient.getQueryData<SettingsView>(SETTINGS_QUERY_KEY);
  if (!current || current === snapshot) {
    queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, stored);
    return;
  }
  const merged = { ...stored } as Record<string, unknown>;
  const currentRecord = current as unknown as Record<string, unknown>;
  const snapshotRecord = snapshot as unknown as Record<string, unknown>;
  for (const key of Object.keys(currentRecord)) {
    if (!settingsValueEqual(currentRecord[key], snapshotRecord[key])) {
      merged[key] = currentRecord[key];
    }
  }
  queryClient.setQueryData<SettingsView>(
    SETTINGS_QUERY_KEY,
    merged as unknown as SettingsView,
  );
}

function readBaseline(): SettingsView | null {
  const baseline = useSettingsBaseline.getState().baseline;
  if (baseline === null) return null;
  try {
    return JSON.parse(baseline) as SettingsView;
  } catch {
    // A malformed baseline must not block saving the new draft.
    return null;
  }
}

function messageSettingsSaved(
  queryClient: QueryClient,
  row: SettingsView,
): void {
  markSettingsSaved();
  // Spotlight toggles live in this row; refresh its status card without
  // refetching the whole settings row (which would clobber the draft).
  void queryClient.invalidateQueries({ queryKey: ["spotlight_status"] });
  window.dispatchEvent(
    new CustomEvent<SettingsView>("mix:settings-saved", { detail: row }),
  );
}

function pushSettingsSaveError(queryClient: QueryClient): void {
  useSettingsSaveStatus.getState().setStatus("error");
  useToasts.getState().push({
    id: "settings-autosave-error",
    message: t("設定を保存できませんでした"),
    action: {
      label: t("再試行"),
      onClick: () => {
        void commitSettingsDraft(queryClient);
      },
    },
    timeoutMs: 10000,
  });
}

/** Restore only the keys the save changed, merged onto the latest row. */
async function undoSettingsPatch(
  queryClient: QueryClient,
  previousRow: SettingsView,
  patch: Partial<SettingsView>,
): Promise<void> {
  const restore: Record<string, unknown> = {};
  const previousRecord = previousRow as unknown as Record<string, unknown>;
  for (const key of Object.keys(patch)) restore[key] = previousRecord[key];
  useSettingsSaveStatus.getState().setStatus("saving");
  try {
    const written = await enqueueSettingsWrite(async () => {
      const latest = await api.getSettings();
      return api.updateSettings({
        ...latest,
        ...(restore as Partial<SettingsView>),
      });
    });
    const row = written ?? ({ ...previousRow, ...patch } as SettingsView);
    // Replace only the restored fields so unrelated unsaved edits survive.
    const current = queryClient.getQueryData<SettingsView>(SETTINGS_QUERY_KEY);
    queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, {
      ...(current ?? row),
      ...(restore as Partial<SettingsView>),
    });
    useSettingsBaseline.getState().setBaseline(JSON.stringify(row));
    recordUxEvent("settings_saved");
    messageSettingsSaved(queryClient, row);
  } catch {
    recordUxEvent("settings_save_failed");
    pushSettingsSaveError(queryClient);
  }
}

/**
 * Write the current draft, if it differs from the last persisted baseline.
 *
 * The write is a patch merged onto a fresh read of the stored row rather than a
 * full snapshot built earlier. Two windows therefore do not clobber unrelated
 * fields, and a queued autosave cannot revert an immediate theme/language
 * write. The queue still serialises the actual `updateSettings` calls.
 */
let settingsCommitInFlight = false;

async function commitSettingsDraft(queryClient: QueryClient): Promise<void> {
  // A page-hide flush can race the debounce timer. The in-flight write already
  // covers the current draft; a newer edit schedules a fresh commit once the
  // baseline advances.
  if (settingsCommitInFlight) return;
  const data = queryClient.getQueryData<SettingsView>(SETTINGS_QUERY_KEY);
  const previousRow = readBaseline();
  if (!data || !previousRow) return;
  const patch = diffSettings(previousRow, data);
  if (Object.keys(patch).length === 0) return;

  settingsCommitInFlight = true;
  useSettingsSaveStatus.getState().setStatus("saving");
  try {
    const written = await enqueueSettingsWrite(async () => {
      const latest = await api.getSettings();
      return api.updateSettings({ ...latest, ...patch });
    });
    const row = written ?? ({ ...previousRow, ...patch } as SettingsView);
    reconcileSettingsDraft(queryClient, data, row);
    useSettingsBaseline.getState().setBaseline(JSON.stringify(row));
    recordUxEvent("settings_saved");
    messageSettingsSaved(queryClient, row);

    // A routine toggle just lights the header badge. Only a consequential
    // change keeps an Undo toast around.
    if (!hasUndoableSettingsChange(patch)) return;
    useToasts.getState().push({
      id: "settings-autosave",
      message: t("設定を保存しました"),
      action: {
        label: t("設定を元に戻す"),
        onClick: () => {
          // `undoSettingsPatch` enqueues its read+write as one task; wrapping
          // it again would deadlock the queue against itself.
          void undoSettingsPatch(queryClient, previousRow, patch);
        },
      },
      timeoutMs: 10000,
    });
  } catch {
    // Keep the draft dirty: the next edit or the retry action tries again.
    // A persistent failure must not toast on every keystroke.
    recordUxEvent("settings_save_failed");
    pushSettingsSaveError(queryClient);
  } finally {
    settingsCommitInFlight = false;
  }
}

/**
 * Persist the shared Settings draft automatically after the user stops editing.
 *
 * This is the single writer for every Settings control. There is one answer to
 * "did it save?" no matter which tab a change came from, and the sidebar no
 * longer needs an unsaved dot.
 *
 * A short debounce keeps typing in a text field (verify command, budget) from
 * firing one write per keystroke. Pending writes are flushed on page hide and
 * on unmount, and a `beforeunload` guard covers the last few hundred
 * milliseconds before quitting (which the old debounce dropped silently).
 */
export function useSettingsAutosave(delayMs = 700): void {
  const queryClient = useQueryClient();
  const { data, loaded, dirty, baseline } = useSettingsForm();
  const timer = useRef<number | null>(null);

  useEffect(() => {
    if (!loaded || !dirty) return;

    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      void commitSettingsDraft(queryClient);
    }, delayMs);

    return () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
        timer.current = null;
      }
    };
  }, [baseline, data, delayMs, dirty, loaded, queryClient]);

  // Best-effort flush when the window is hidden or torn down. The close path
  // itself is native, so `beforeunload` may be ignored by the WebView; pagehide
  // is the reliable signal we do get.
  useEffect(() => {
    const flush = () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
        timer.current = null;
      }
      void commitSettingsDraft(queryClient);
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") flush();
    };
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", onVisibilityChange);
      flush();
    };
  }, [queryClient]);

  // Native close dialogs often ignore this, but it still protects reloads and
  // any WebView that honours it.
  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);
}

/** Persist a change immediately, bypassing the debounce.
 *
 * Used for controls that both change the UI on click and must not wait for the
 * 700 ms autosave timer (theme, language, onboarding completion). Ordinary
 * Settings controls only call `patch`; `useSettingsAutosave` writes those. */
export function useApplySettings() {
  const queryClient = useQueryClient();
  const setBaseline = useSettingsBaseline((state) => state.setBaseline);
  const apply = async (patch: Partial<SettingsView>): Promise<void> => {
    const cached = queryClient.getQueryData<SettingsView>(SETTINGS_QUERY_KEY);
    let persisted: SettingsView | null = null;
    useSettingsSaveStatus.getState().setStatus("saving");
    try {
      // Immediate-save controls must not promote unrelated edits from the
      // Settings form. That form lives in this same cache entry, so merging
      // the patch onto `cached` used to persist a half-typed verify command
      // with a theme change and then mark it saved. Read the persisted row for
      // the write and keep the draft in the cache untouched.
      try {
        persisted = await api.getSettings();
      } catch {
        // The update itself may still succeed (e.g. a transient read error);
        // fall back to the cache rather than turning a theme click into a
        // no-op.
        persisted = cached ?? (await api.getSettings());
      }
      const saved: SettingsView = { ...persisted, ...patch };
      const next: SettingsView = { ...(cached ?? persisted), ...patch };
      queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, next);
      const written = await api.updateSettings(saved);
      // Report the row that was actually written, not the draft, so unrelated
      // unsaved edits stay flagged as unsaved.
      const persistedRow = written ?? saved;
      setBaseline(JSON.stringify(persistedRow));
      recordUxEvent("settings_saved");
      messageSettingsSaved(queryClient, persistedRow);
    } catch (err) {
      // Revert the optimistic value but keep the rest of the draft. Refetching
      // here used to wipe a half-typed field in another tab along with the
      // failed change.
      console.error("saving settings failed", err);
      const revert = cached ?? persisted;
      if (revert) {
        queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, revert);
      } else {
        await queryClient.invalidateQueries({
          queryKey: SETTINGS_QUERY_KEY,
        });
      }
      recordUxEvent("settings_save_failed");
      useSettingsSaveStatus.getState().setStatus("error");
      useToasts.getState().push({
        id: "settings-apply-error",
        message: t("設定を保存できませんでした"),
        action: {
          label: t("再試行"),
          onClick: () => {
            void enqueueSettingsWrite(() => apply(patch));
          },
        },
        timeoutMs: 10000,
      });
    }
  };
  return (patch: Partial<SettingsView>): Promise<void> =>
    enqueueSettingsWrite(() => apply(patch));
}
