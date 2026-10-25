import { useEffect, useState } from "react";
import { Minus, Square, Copy, X } from "lucide-react";
import { t } from "../../lib/i18n";

export function WindowControls() {
  const [isMac, setIsMac] = useState(true);
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    // macOS では OS の Traffic Light ボタン（Overlay）を使うため表示しない。
    // Windows / Linux では dataset.platform が "other" になる。
    const platform = document.documentElement.dataset.platform;
    setIsMac(platform === "macos");

    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const appWindow = getCurrentWindow();
        const max = await appWindow.isMaximized();
        setIsMaximized(max);
        unlisten = await appWindow.onResized(async () => {
          try {
            setIsMaximized(await appWindow.isMaximized());
          } catch {
            // noop
          }
        });
      } catch {
        // Tauri 外（Web ブラウザプレビューやテスト環境）では無視
      }
    })();

    return () => {
      unlisten?.();
    };
  }, []);

  if (isMac) {
    return null;
  }

  const handleMinimize = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().minimize();
    } catch {
      // noop
    }
  };

  const handleToggleMaximize = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      await win.toggleMaximize();
      setIsMaximized(await win.isMaximized());
    } catch {
      // noop
    }
  };

  const handleClose = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().close();
    } catch {
      // noop
    }
  };

  return (
    <div
      className="flex h-titlebar shrink-0 items-center -me-3 ms-1"
      role="toolbar"
      aria-label={t("ウィンドウ操作")}
    >
      <button
        type="button"
        tabIndex={-1}
        onClick={handleMinimize}
        aria-label={t("最小化")}
        title={t("最小化")}
        className="inline-flex h-titlebar w-11 items-center justify-center text-muted transition-colors hover:bg-hover hover:text-fg focus:outline-none"
      >
        <Minus size={13} aria-hidden="true" />
      </button>
      <button
        type="button"
        tabIndex={-1}
        onClick={handleToggleMaximize}
        aria-label={isMaximized ? t("元のサイズに戻す") : t("最大化")}
        title={isMaximized ? t("元のサイズに戻す") : t("最大化")}
        className="inline-flex h-titlebar w-11 items-center justify-center text-muted transition-colors hover:bg-hover hover:text-fg focus:outline-none"
      >
        {isMaximized ? (
          <Copy size={12} className="rotate-180" aria-hidden="true" />
        ) : (
          <Square size={12} aria-hidden="true" />
        )}
      </button>
      <button
        type="button"
        tabIndex={-1}
        onClick={handleClose}
        aria-label={t("閉じる")}
        title={t("閉じる")}
        className="inline-flex h-titlebar w-11 items-center justify-center text-muted transition-colors hover:bg-[#e81123] hover:text-white focus:outline-none"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  );
}
