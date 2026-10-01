import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { PENDING_APPROVALS_POLL_MS } from "@/lib/settings";
import { useUi } from "@/lib/ui";
import type { ProjectView } from "@/lib/types";

/** How long a project list is considered fresh.
 *
 * Projects change when the user adds or edits one, and every editor action
 * invalidates the key anyway. The constant only matters because the query used
 * to be written out five times with two different `staleTime`s: the observers
 * that declared 30s and the three that left it at zero shared one cache entry,
 * and the shortest `staleTime` among the mounted observers is the one that
 * decided when a refetch actually happened.
 */
export const PROJECTS_STALE_MS = 30_000;

/** How often the working tree is re-read. */
export const GIT_STATUS_POLL_MS = 15_000;

/** How long a provider list is considered fresh. See `useProviders`. */
export const PROVIDERS_STALE_MS = 30_000;

export const PROJECTS_QUERY_KEY = ["projects"] as const;
export const CONVERSATIONS_QUERY_KEY = ["conversations"] as const;
export const PENDING_APPROVALS_QUERY_KEY = ["pending_approvals"] as const;
export const PROVIDERS_QUERY_KEY = ["providers"] as const;
export const PROVIDER_PRESETS_QUERY_KEY = ["provider_presets"] as const;
export const MODELS_QUERY_KEY = ["models"] as const;

export const gitStatusKey = (root: string) => ["git_status", root] as const;

/** Every configured provider.
 *
 * The key was written out by hand at each call site, which meant three
 * independent observers shared one cache entry while disagreeing about
 * `staleTime` — and the onboarding wizard joined the list. A provider list only
 * changes when a provider is created, updated or deleted, and every such
 * mutation invalidates the key, so the default zero staleness just buys a
 * refetch on mount for panes that are otherwise idle.
 */
export function useProviders() {
  return useQuery({
    queryKey: PROVIDERS_QUERY_KEY,
    queryFn: api.listProviders,
    staleTime: PROVIDERS_STALE_MS,
  });
}

/** The built-in provider catalog. Fixed at build time — it never changes for a
 * given install, so it is fetched once per session. */
export function useProviderPresets() {
  return useQuery({
    queryKey: PROVIDER_PRESETS_QUERY_KEY,
    queryFn: api.providerPresets,
    staleTime: Infinity,
  });
}

/** Models discovered on every provider. */
export function useModels() {
  return useQuery({ queryKey: MODELS_QUERY_KEY, queryFn: () => api.listModels() });
}

export function useProjects() {
  return useQuery({
    queryKey: PROJECTS_QUERY_KEY,
    queryFn: api.listProjects,
    staleTime: PROJECTS_STALE_MS,
  });
}

export function useConversations() {
  return useQuery({ queryKey: CONVERSATIONS_QUERY_KEY, queryFn: api.listConversations });
}

/** The project currently driving the file tree / diff scope. */
export function useActiveProject(): ProjectView | null {
  const activeProjectId = useUi((s) => s.activeProjectId);
  const projects = useProjects();
  if (!activeProjectId) return null;
  return projects.data?.find((p) => p.id === activeProjectId) ?? null;
}

/** Absolute folder of the active project (`null` when unset / missing). */
export function useActiveRoot(): string | null {
  return useActiveProject()?.rootPath ?? null;
}

/** Tool calls waiting for an approve / deny answer.
 *
 * One interval for every observer: the approvals pane used to poll twice as
 * often as the sidebar and settings tab, and react-query lets any one observer
 * restart the shared timer, so the pane's rate won.
 */
export function usePendingApprovals() {
  return useQuery({
    queryKey: PENDING_APPROVALS_QUERY_KEY,
    queryFn: api.listPendingApprovals,
    refetchInterval: PENDING_APPROVALS_POLL_MS,
  });
}

/** The working tree of one folder.
 *
 * Both the file tree and the changes panel want this, and both used to run
 * their own 15s timer — two `git status` subprocesses per interval — while
 * sitting on one cache entry.
 */
export function useGitStatus(root: string | null) {
  return useQuery({
    queryKey: gitStatusKey(root ?? ""),
    queryFn: () => api.gitStatus(root!),
    enabled: !!root,
    refetchInterval: GIT_STATUS_POLL_MS,
  });
}
