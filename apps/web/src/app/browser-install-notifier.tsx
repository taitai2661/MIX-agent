import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/app/api";
import { t } from "@/app/i18n";
import { useToast, type ToastType } from "@/components/toast";

type InstallStatus = "not_installed" | "installing" | "ready" | "failed";

interface InstallNotice {
  type: ToastType;
  title: string;
  message: string;
}

/** The popup shown when a background install starts or finishes. */
export function browserInstallNotice(
  previous: string | null,
  next: string,
): InstallNotice | null {
  if (next === "installing" && previous !== "installing") {
    return {
      type: "info",
      title: t("Chromiumをインストールしています"),
      message: t("バックグラウンドで続行します。完了すると通知します。"),
    };
  }
  if (next === "ready" && previous === "installing") {
    return {
      type: "success",
      title: t("Chromiumのインストールが完了しました"),
      message: t("ブラウザ機能を利用できます。"),
    };
  }
  if (next === "failed" && previous === "installing") {
    return {
      type: "error",
      title: t("Chromiumのインストールに失敗しました"),
      message: t("設定画面から再試行できます。"),
    };
  }
  return null;
}

/**
 * Keeps watching `/settings` while Chromium installs in the background and
 * emits a toast when the install starts, succeeds, or fails.
 *
 * Setup and the Browser settings pane start the install outside this component,
 * so it must own the polling rather than rely on that page staying mounted.
 */
export function BrowserInstallNotifier() {
  const { addToast } = useToast();
  const previous = useRef<string | null>(null);
  const query = useQuery({
    queryKey: ["/settings"],
    queryFn: () => api<{ data: Record<string, unknown> }>("/settings"),
    refetchInterval: (query) =>
      query.state.data?.data.browser_install_status === "installing"
        ? 3000
        : false,
    refetchIntervalInBackground: true,
  });
  const raw = query.data?.data.browser_install_status;
  const status: InstallStatus | null =
    raw === "not_installed" ||
    raw === "installing" ||
    raw === "ready" ||
    raw === "failed"
      ? raw
      : null;

  useEffect(() => {
    if (!status) return;
    const notice = browserInstallNotice(previous.current, status);
    previous.current = status;
    if (notice) addToast(notice);
  }, [status, addToast]);

  return null;
}
