/// Shared backend-health observer.
///
/// Both the titlebar dot and the error banner read the same query key, so a
/// single 15s poll serves the whole shell. Keeping the interval here (rather
/// than in one component) means the two surfaces cannot drift apart.

import { useQuery } from "@tanstack/react-query";
import { api } from "./api";

export const HEALTH_QUERY_KEY = ["health"] as const;
export const HEALTH_POLL_MS = 15_000;

export function useHealth() {
  return useQuery({
    queryKey: HEALTH_QUERY_KEY,
    queryFn: api.health,
    refetchInterval: HEALTH_POLL_MS,
  });
}
