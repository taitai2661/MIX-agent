import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { api } from "./api";
import { errorMessage } from "./errors";
import type { UpdateInfo, UpdateProgress } from "./types";

/** Shared cache key so the header badge and the Settings pane agree on the
 *  result of one launch check instead of each issuing their own request. */
export const UPDATE_QUERY_KEY = ["update-check"] as const;

export type UpdatePhase = "idle" | "downloading" | "ready" | "error";

interface UpdateStore {
  /** What the update flow is doing right now. */
  phase: UpdatePhase;
  /** A verified payload waiting for the next normal quit. */
  pending: UpdateInfo | null;
  progress: UpdateProgress | null;
  error: string | null;
  /** True once the backend has answered `pending_update` (or that command is
   *  unavailable in an older build). Auto-download waits for this so it cannot
   *  race the cache restore and download a release that is already staged. */
  hydrated: boolean;
}

export const useUpdateStore = create<UpdateStore>(() => ({
  phase: "idle",
  pending: null,
  progress: null,
  error: null,
  hydrated: false,
}));

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

/** Drop a stale download error before a fresh manual check. A ready payload
 *  and an in-flight download keep their phase. */
export function clearUpdateError(): void {
  const state = useUpdateStore.getState();
  useUpdateStore.setState({
    error: null,
    phase: state.phase === "error" ? "idle" : state.phase,
  });
}

/**
 * Download and stage the update, without installing it.
 *
 * Safe to call from both the automatic launch path and the manual button: the
 * backend rejects a second download while one is in flight, and the store
 * guard keeps React's StrictMode double-effect from starting two requests.
 */
export async function downloadPendingUpdate(): Promise<void> {
  const current = useUpdateStore.getState();
  if (current.phase === "downloading" || current.pending) return;
  useUpdateStore.setState({
    phase: "downloading",
    progress: null,
    error: null,
  });
  try {
    const info = await api.downloadUpdate();
    useUpdateStore.setState({
      phase: "ready",
      pending: info,
      progress: null,
      error: null,
    });
  } catch (error) {
    useUpdateStore.setState({
      phase: "error",
      progress: null,
      error: errorMessage(error),
    });
  }
}

/**
 * App-wide update lifecycle: restore any staged payload, stream download
 * progress, and start the background download when the settings allow it.
 *
 * Call exactly once from the app root. Both the header and the Settings pane
 * read the same Zustand store, so a download started here keeps running when
 * the pane unmounts.
 */
export function useUpdateBootstrap({
  autoCheck,
  autoDownload,
}: {
  autoCheck: boolean;
  autoDownload: boolean;
}) {
  const check = useUpdateCheck(autoCheck);

  useEffect(() => {
    let cancelled = false;
    let unlistenProgress: (() => void) | undefined;
    let unlistenReady: (() => void) | undefined;

    void (async () => {
      try {
        const [progress, ready] = await Promise.all([
          listen<UpdateProgress>("update://progress", (event) => {
            if (cancelled) return;
            // A late progress event must not move a ready payload back to
            // "downloading".
            if (useUpdateStore.getState().pending) return;
            useUpdateStore.setState({
              phase: "downloading",
              progress: event.payload,
            });
          }),
          listen<UpdateInfo>("update://ready", (event) => {
            if (cancelled) return;
            useUpdateStore.setState({
              phase: "ready",
              pending: event.payload,
              progress: null,
              error: null,
            });
          }),
        ]);
        if (cancelled) {
          progress();
          ready();
        } else {
          unlistenProgress = progress;
          unlistenReady = ready;
        }
      } catch {
        // Older builds without these events simply never emit them; the
        // command response still drives the store.
      }
    })();

    return () => {
      cancelled = true;
      unlistenProgress?.();
      unlistenReady?.();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void api
      .pendingUpdate()
      .then((info) => {
        if (cancelled) return;
        useUpdateStore.setState((state) =>
          info
            ? {
                hydrated: true,
                phase: "ready",
                pending: info,
                progress: null,
                error: null,
              }
            : { hydrated: true, phase: state.phase },
        );
      })
      .catch(() => {
        // The command is new in this build; an older backend must not block
        // the automatic download path.
        if (!cancelled) useUpdateStore.setState({ hydrated: true });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const phase = useUpdateStore((state) => state.phase);
  const hydrated = useUpdateStore((state) => state.hydrated);

  useEffect(() => {
    if (!autoCheck || !autoDownload || !hydrated) return;
    if (phase !== "idle") return;
    if (!check.isSuccess || !check.data) return;
    // A Linux .deb install is managed by the package manager; the backend
    // refuses the download, so do not surface that as an automatic error.
    if (!check.data.installable) return;
    void downloadPendingUpdate();
  }, [autoCheck, autoDownload, hydrated, phase, check.isSuccess, check.data]);

  return check;
}
