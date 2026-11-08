import { useEffect, useState } from "react";
import { Minus, Square, Copy, X } from "lucide-react";
import { t } from "../../lib/i18n";

export function WindowControls() {
  // Start hidden: on macOS the OS traffic lights are used, and a Tauri WebView
  // can report a Windows-ish UA before `plugin-os` resolves. Showing our own
  // buttons first would overlap the traffic lights.
  const [isMac, setIsMac] = useState(true);
  // `null` until the native window answers. A decorated window (the Windows or
  // Linux build running with the stock decorations, e.g. an older installer or
  // a `--config` that skipped the platform override) already draws minimize /
  // maximize / close, so landing on `false` after a paint would flash a second
  // set on top. Stay hidden until Tauri says the window is undecorated.
  const [decorated, setDecorated] = useState<boolean | null>(null);
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    // macOS では OS の Traffic Light ボタン（Overlay）を使うため表示しない。
    // Windows / Linux では dataset.platform が "windows" / "linux" になる。
    // `main.tsx` writes a UA guess first and `App` replaces it with the
    // authoritative `plugin-os` value after mount. Reading once locked in the
    // provisional "other" and left our close buttons on top of the macOS
    // traffic lights, so follow later corrections too.
    const sync = () => {
      setIsMac(document.documentElement.dataset.platform === "macos");
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-platform"],
    });

    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const appWindow = getCurrentWindow();
        const max = await appWindow.isMaximized();
        setIsMaximized(max);
        // Windows/Linux normally run with `decorations: false` (see
        // tauri.windows.conf.json / tauri.linux.conf.json) because the shell
        // draws its own title bar. If the window is decorated instead, the OS
        // chrome is the authoritative set and ours would duplicate it.
        setDecorated(await appWindow.isDecorated());
        unlisten = await appWindow.onResized(async () => {
          try {
            setIsMaximized(await appWindow.isMaximized());
          } catch {
            // noop
          }
        });
      } catch {
        // Tauri 外（Web ブラウザプレビューやテスト環境）では無視。ネイティブの
        // ウィンドウ装飾も無いので、アプリ内のボタンをそのまま使う。
        setDecorated(false);
      }
    })();

    return () => {
      observer.disconnect();
      unlisten?.();
    };
  }, []);

  if (isMac || decorated !== false) {
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
