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
  auto_update: true,
  telemetry_consent: false,
  language: null,
  theme: null,
  projects_root: null,
  project_doc_max_bytes: null,
  project_doc_fallback_filenames: [],
  onboarding_completed: null,
};

/** Shared access to the settings draft: one query, one cache entry, and a
 * `patch` that refuses to seed the cache with defaults before the load
 * finishes (which would make a later Save persist them). */
export function useSettingsForm() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: SETTINGS_QUERY_KEY, queryFn: api.getSettings });
  const loaded = query.data !== undefined;
  const data = query.data ?? DEFAULT_SETTINGS;
  const patch = (next: Partial<SettingsView>) => {
    if (!loaded) return;
    queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, { ...data, ...next });
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
export function useApplySettings() {
  const queryClient = useQueryClient();
  return async (patch: Partial<SettingsView>): Promise<void> => {
    try {
      const base =
        queryClient.getQueryData<SettingsView>(SETTINGS_QUERY_KEY) ?? (await api.getSettings());
      const next: SettingsView = { ...base, ...patch };
      queryClient.setQueryData<SettingsView>(SETTINGS_QUERY_KEY, next);
      await api.updateSettings(next);
    } catch (err) {
      // The optimistic value must not survive a failed write: drop it and let
      // the query refetch the truth.
      console.error("saving settings failed", err);
      await queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEY });
    }
  };
}
