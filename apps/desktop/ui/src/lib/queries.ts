import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { PENDING_APPROVALS_POLL_MS } from "@/lib/settings";
import { useUi } from "@/lib/ui";
import type { ProjectView, SpotlightStatus } from "@/lib/types";

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
/** Page cache shares the `conversations` prefix, so every existing
 *  `invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY })` also refreshes
 *  the paginated sidebar without touching each call site. */
export const CONVERSATION_PAGES_QUERY_KEY = [
  ...CONVERSATIONS_QUERY_KEY,
  "pages",
] as const;
export const PENDING_APPROVALS_QUERY_KEY = ["pending_approvals"] as const;
export const PROVIDERS_QUERY_KEY = ["providers"] as const;
export const PROVIDER_PRESETS_QUERY_KEY = ["provider_presets"] as const;
export const MODELS_QUERY_KEY = ["models"] as const;
export const LOCAL_RUNTIMES_QUERY_KEY = ["local_runtimes"] as const;
export const LOCAL_JOBS_QUERY_KEY = ["local_jobs"] as const;
export const SPOTLIGHT_STATUS_QUERY_KEY = ["spotlight_status"] as const;

export const gitStatusKey = (root: string, allUntracked = false) =>
  ["git_status", root, allUntracked] as const;

/** Polling stops while the WebView is hidden (for example, minimised to the
 *  tray). A hidden window has no visible file decorations to refresh. */
export const visiblePoll = (ms: number) => (): number | false =>
  typeof document !== "undefined" && document.visibilityState === "hidden"
    ? false
    : ms;

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
  return useQuery({
    queryKey: MODELS_QUERY_KEY,
    queryFn: () => api.listModels(),
    staleTime: PROVIDERS_STALE_MS,
  });
}

/** Lifecycle state of every discovered local runtime.
 *
 * The manager also emits `local_runtime_event`; the polling interval is the
 * fallback for a missed event (a hidden WebView, a runtime that dies while the
 * pane is closed) rather than the primary update path.
 */
export function useLocalRuntimes() {
  return useQuery({
    queryKey: LOCAL_RUNTIMES_QUERY_KEY,
    queryFn: api.localRuntimeSnapshot,
    staleTime: 0,
    refetchInterval: visiblePoll(10_000),
  });
}

export function useProjects() {
  return useQuery({
    queryKey: PROJECTS_QUERY_KEY,
    queryFn: api.listProjects,
    staleTime: PROJECTS_STALE_MS,
  });
}

export const CONVERSATION_QUERY_KEY = [
  ...CONVERSATIONS_QUERY_KEY,
  "detail",
] as const;
export const CONVERSATION_STATS_QUERY_KEY = [
  ...CONVERSATIONS_QUERY_KEY,
  "stats",
] as const;

/** One thread by id. Used for titles/selection so callers no longer depend on
 *  the bounded `list_conversations` list. */
export function useConversation(id: string | null | undefined) {
  return useQuery({
    queryKey: [...CONVERSATION_QUERY_KEY, id ?? ""],
    queryFn: () => api.getConversation(id!),
    enabled: !!id,
    staleTime: PROJECTS_STALE_MS,
  });
}

/** Newest thread in any of `modes` (the surface's default selection). */
export function useLatestConversation(modes: string[]) {
  return useQuery({
    queryKey: [...CONVERSATIONS_QUERY_KEY, "latest", modes.join(",")],
    queryFn: () => api.latestConversation(modes),
    staleTime: PROJECTS_STALE_MS,
  });
}

/** Threads linked to one project, newest first. */
export function useConversationsForProject(
  projectId: string | null | undefined,
) {
  return useQuery({
    queryKey: [...CONVERSATIONS_QUERY_KEY, "by-project", projectId ?? ""],
    queryFn: () => api.listConversationsForProject(projectId!),
    enabled: !!projectId,
    staleTime: PROJECTS_STALE_MS,
  });
}

/** Server-side title search across every thread. */
export function useConversationSearch(query: string) {
  const trimmed = query.trim();
  return useQuery({
    queryKey: [...CONVERSATIONS_QUERY_KEY, "search", trimmed],
    queryFn: () => api.searchConversations(trimmed),
    enabled: trimmed.length > 0,
    staleTime: 30_000,
  });
}

/** Cheap aggregate counts for the welcome card. */
export function useConversationStats() {
  return useQuery({
    queryKey: CONVERSATION_STATS_QUERY_KEY,
    queryFn: api.conversationStats,
    staleTime: PROJECTS_STALE_MS,
  });
}

/** First page only, for the palette's idle state. */
export function useRecentConversations(limit = 50, enabled = true) {
  return useQuery({
    queryKey: [...CONVERSATIONS_QUERY_KEY, "recent", limit],
    queryFn: () => api.listConversationsPage(limit, null),
    enabled,
    staleTime: PROJECTS_STALE_MS,
  });
}

/** Paginated thread list for the sidebar.
 *
 *  The first page paints immediately; older pages load when the user scrolls
 *  near the end (or presses the explicit button). Keeping the cursor in the
 *  page param means a thread renamed or moved between requests cannot make the
 *  next page repeat a row. */
export function useConversationPages() {
  return useInfiniteQuery({
    queryKey: CONVERSATION_PAGES_QUERY_KEY,
    queryFn: ({ pageParam }) =>
      api.listConversationsPage(
        CONVERSATION_PAGE_SIZE,
        pageParam as { cursor: string; id: string } | null,
      ),
    initialPageParam: null as { cursor: string; id: string } | null,
    getNextPageParam: (last) => {
      const tail = last.items[last.items.length - 1];
      return last.has_more && tail
        ? { cursor: tail.cursor, id: tail.id }
        : undefined;
    },
    staleTime: PROJECTS_STALE_MS,
  });
}

/** Matches the backend default. Kept here so the UI and the command agree. */
export const CONVERSATION_PAGE_SIZE = 50;

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
    refetchInterval: visiblePoll(PENDING_APPROVALS_POLL_MS),
  });
}

/** The working tree of one folder.
 *
 * Both the file tree and the changes panel want this, and both used to run
 * their own 15s timer — two `git status` subprocesses per interval — while
 * sitting on one cache entry.
 */
export function useGitStatus(root: string | null, allUntracked = false) {
  return useQuery({
    queryKey: gitStatusKey(root ?? "", allUntracked),
    queryFn: () => api.gitStatus(root!, allUntracked),
    enabled: !!root,
    refetchInterval: visiblePoll(GIT_STATUS_POLL_MS),
  });
}

/** Spotlight queue status for Settings. Short interval while the tab is open;
 *  indexing runs in the background and the counts change without a mutation. */
export function useSpotlightStatus(enabled = true) {
  return useQuery<SpotlightStatus>({
    queryKey: SPOTLIGHT_STATUS_QUERY_KEY,
    queryFn: api.spotlightStatus,
    enabled,
    refetchInterval: visiblePoll(5_000),
  });
}
