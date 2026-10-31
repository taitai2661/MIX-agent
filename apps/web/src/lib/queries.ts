import { api, type Row } from "@/app/api";
import { useQuery } from "@tanstack/react-query";

/**
 * Shared query keys and hooks.
 *
 * These strings used to be duplicated in ten files (`["/settings"]`,
 * `["/conversations"]`, …). Centralising them means an invalidation cannot
 * miss a reader because one call site spelled the key differently, and the
 * `as const` tuples keep TanStack's inference.
 */
export const queryKeys = {
  settings: ["/settings"] as const,
  conversations: ["/conversations"] as const,
  conversationFolders: ["/conversation-folders"] as const,
  projects: ["/projects"] as const,
  providers: ["/providers"] as const,
  models: ["/models"] as const,
  tools: ["/tools"] as const,
  mcpConnections: ["/mcp/connections"] as const,
  messages: (conversationId: string) => ["messages", conversationId] as const,
  toolCalls: (conversationId: string) =>
    ["tool-calls", conversationId] as const,
  run: (runId: string) => ["run", runId] as const,
  runEvents: (runId: string) => ["run_events", runId] as const,
  checkpoints: (runId: string) => ["checkpoints", runId] as const,
  browserFrames: (runId: string) => ["browser-frames", runId] as const,
};

export const SETTINGS_QUERY_KEY = queryKeys.settings;

/** The single `/settings` reader; the payload shape differs per caller. */
export function useSettings<T = Row>() {
  return useQuery<T>({
    queryKey: queryKeys.settings,
    queryFn: () => api<T>("/settings"),
  });
}
