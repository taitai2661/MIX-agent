import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import type { BrowserProfileView, BrowserStatus } from "@/lib/types";

type ReportError = (error: unknown) => void;

/**
 * Backend status, profiles and the managed-Chromium lifecycle.
 *
 * Install progress arrives twice: the Rust side emits `browser://install`, and
 * this hook polls while a stage is active. The event keeps the bar responsive;
 * the poll converges after a missed event or a reopened pane. Terminal events
 * trigger a full refresh because installing also changes `source`,
 * `executable` and whether the agent sees the `browser` tool.
 */
export function useBrowserStatus(reportError: ReportError) {
  const [status, setStatus] = useState<BrowserStatus | null>(null);
  const [profiles, setProfiles] = useState<BrowserProfileView[]>([]);
  // `null` until the first status read lands. The pane used to show a literal
  // "…" inside a badge, which read as a status value rather than as "loading".
  const [loading, setLoading] = useState(true);
  const [installBusy, setInstallBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [s, p] = await Promise.all([
        api.browserStatus(),
        api.browserProfiles(),
      ]);
      setStatus(s);
      setProfiles(p);
      // Do not jump to another profile just because this workspace has not
      // been materialised yet. The old auto-switch sent a manual sign-in to a
      // different conversation's cookies.
    } catch (e) {
      reportError(e);
    } finally {
      setLoading(false);
    }
  }, [reportError]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // A progress event landed; merge it into the cached install block. The
  // terminal states also change what the backend resolves, so refresh them.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void api
      .browserInstallEvent((install) => {
        setStatus((current) => (current ? { ...current, install } : current));
        if (
          install.status === "ready" ||
          install.status === "failed" ||
          install.status === "cancelled"
        ) {
          void refresh();
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Older shells may not expose the event; the poll below still works.
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refresh]);

  const installStatus = status?.install.status;
  const installActive =
    installStatus === "downloading" ||
    installStatus === "verifying" ||
    installStatus === "extracting";

  // The install is a background job in Rust. Poll while it runs so progress
  // survives a missed event and so the pane converges after reopening.
  useEffect(() => {
    if (!installActive) return;
    const id = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(id);
  }, [installActive, refresh]);

  const startInstall = useCallback(
    async (force: boolean) => {
      setInstallBusy(true);
      try {
        const next = await api.browserInstall(force);
        setStatus((current) =>
          current ? { ...current, install: next } : current,
        );
      } catch (e) {
        reportError(e);
      } finally {
        setInstallBusy(false);
        await refresh();
      }
    },
    [refresh, reportError],
  );

  const cancelInstall = useCallback(async () => {
    try {
      const next = await api.browserInstallCancel();
      setStatus((current) =>
        current ? { ...current, install: next } : current,
      );
    } catch (e) {
      reportError(e);
    } finally {
      await refresh();
    }
  }, [refresh, reportError]);

  const removeChromium = useCallback(async () => {
    try {
      await api.browserUninstall();
      await refresh();
    } catch (e) {
      reportError(e);
    }
  }, [refresh, reportError]);

  return {
    status,
    profiles,
    loading,
    installBusy,
    installActive,
    refresh,
    startInstall,
    cancelInstall,
    removeChromium,
  };
}
