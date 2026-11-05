import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { SettingsView } from "@/lib/types";

export const SETTINGS_QUERY_KEY = ["settings"] as const;

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
  // SQLCipher is not wired yet; the only honest value is `false`.
  database_encryption: false,
  onboarding_completed: null,
  // The backend defaults this to `true` too: auto-check is one manifest
  // request and the payload is signature-verified before installation.
  auto_check_updates: true,
  local_scan_on_start: true,
  local_stop_on_exit: true,
  // Spawning `ollama serve` for a user who manages it as a service was a
  // surprise; the Providers pane has an explicit Start button.
  local_auto_start: false,
  local_keep_alive: null,
  local_context_window: null,
  local_prefer: false,
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
  const patch = (next: Partial<SettingsView>) => {
    if (!loaded) return;
    queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, {
      ...data,
      ...next,
    });
  };
  // Settled, not loaded: a failed `get_settings` leaves `loaded` false forever,
  // and a caller gating on it would hang rather than fall back to defaults.
  return { data, loaded, settled: !query.isPending, patch, query };
}

/** Persist a change immediately instead of waiting for the header Save button.
 *
 * Used for controls that apply on click (language, theme): without this they
 * lived only in the local store and were thrown away on the next launch, while
 * the settings row kept a different value — two sources of truth that used to
 * fight each other at boot. */
/** Immediate-save writes are serialized: two quick clicks (theme then
 *  language) each read the persisted row first, and running them in parallel
 *  let the second write a snapshot from before the first landed. */
let settingsWriteQueue: Promise<void> = Promise.resolve();

export function useApplySettings() {
  const queryClient = useQueryClient();
  const apply = async (patch: Partial<SettingsView>): Promise<void> => {
    try {
      const cached = queryClient.getQueryData<SettingsView>(SETTINGS_QUERY_KEY);
      // Immediate-save controls must not promote unrelated edits from the
      // Settings form. That form lives in this same cache entry, so merging
      // the patch onto `cached` used to persist a half-typed verify command
      // with a theme change and then mark it saved. Read the persisted row for
      // the write and keep the draft in the cache untouched.
      let persisted: SettingsView;
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
      await api.updateSettings(saved);
      // The Settings pane snapshots the form to decide between "保存しました"
      // and "未保存の変更". Report the row that was actually written, not the
      // draft, so unrelated unsaved edits stay flagged as unsaved.
      window.dispatchEvent(
        new CustomEvent<SettingsView>("mix:settings-saved", { detail: saved }),
      );
    } catch (err) {
      // The optimistic value must not survive a failed write: drop it and let
      // the query refetch the truth.
      console.error("saving settings failed", err);
      await queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEY });
    }
  };
  return (patch: Partial<SettingsView>): Promise<void> => {
    // `apply` swallows its own errors; the rejection branch also keeps a
    // future write from being skipped if it ever does throw.
    settingsWriteQueue = settingsWriteQueue.then(
      () => apply(patch),
      () => apply(patch),
    );
    return settingsWriteQueue;
  };
}
