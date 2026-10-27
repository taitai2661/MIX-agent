import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import type { UpdateInfo } from "./types";

/** Shared cache key so the header badge and the Settings pane agree on the
 *  result of one launch check instead of each issuing their own request. */
export const UPDATE_QUERY_KEY = ["update-check"] as const;

/**
 * Ask the configured updater endpoint whether a newer build exists.
 *
 * `api.checkForUpdate` resolves `null` when this build is current, so `data`
 * being `null` is a successful "up to date", not "not checked yet"
 * (`isFetched` distinguishes the two).
 *
 * Manual `refetch()` still works when `enabled` is false — that is how the
 * Updates pane's button checks on demand after the user turns the launch
 * check off.
 */
export function useUpdateCheck(enabled: boolean) {
  return useQuery<UpdateInfo | null>({
    queryKey: UPDATE_QUERY_KEY,
    queryFn: api.checkForUpdate,
    enabled,
    // A manifest check is cheap, but not free; five minutes keeps the header
    // from re-requesting every time Settings mounts.
    staleTime: 5 * 60_000,
    // Offline is not an error worth retrying in a loop.
    retry: false,
  });
}
