import { useEffect, useRef } from "react";
import { Globe } from "lucide-react";
import { t } from "@/lib/i18n";

/**
 * Placeholder for the native child WebView.
 *
 * React must leave a hole here: the real page is a sibling WebView owned by
 * Tauri. This component only reports the hole's logical rectangle and hides
 * the native view while a menu/dialog needs to paint above it.
 */
export function BrowserSurface({
  setBounds,
  visible = true,
}: {
  setBounds: (bounds: {
    x: number;
    y: number;
    width: number;
    height: number;
    visible: boolean;
  }) => void;
  visible?: boolean;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const frame = useRef<number | null>(null);
  const lastKey = useRef("");

  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const report = () => {
      frame.current = null;
      const rect = node.getBoundingClientRect();
      if (rect.width < 1 || rect.height < 1) return;
      const key = `${rect.x}:${rect.y}:${rect.width}:${rect.height}:${visible}`;
      if (key === lastKey.current) return;
      lastKey.current = key;
      setBounds({
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        visible,
      });
    };
    const schedule = () => {
      if (frame.current !== null) return;
      frame.current = window.requestAnimationFrame(report);
    };
    report();
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(schedule);
    observer?.observe(node);
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      if (frame.current !== null) window.cancelAnimationFrame(frame.current);
      const rect = node.getBoundingClientRect();
      setBounds({
        x: rect.x,
        y: rect.y,
        width: Math.max(1, rect.width),
        height: Math.max(1, rect.height),
        visible: false,
      });
    };
  }, [setBounds, visible]);

  return (
    <div
      ref={ref}
      data-browser-surface
      className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden bg-inset"
    >
      <div className="pointer-events-none flex max-w-64 flex-col items-center gap-1.5 p-6 text-center text-xs text-faint">
        <Globe size={18} />
        <span>{t("ネイティブブラウザを読み込んでいます…")}</span>
      </div>
    </div>
  );
}
