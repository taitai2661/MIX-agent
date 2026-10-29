import { Play, RefreshCw, Square } from "lucide-react";
import { t } from "@/lib/i18n";
import { Row } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { framePoint } from "./framePoint";
import type { useLiveFrame } from "./useLiveFrame";

export type LiveBrowser = ReturnType<typeof useLiveFrame>;

interface BrowserViewportProps {
  live: LiveBrowser;
  /** Fill the parent's remaining height instead of using a fixed max-height. */
  fill?: boolean;
  /** Hide the live/refresh toggle when the parent renders its own toolbar. */
  hideToolbar?: boolean;
}

/**
 * The shared live surface for the settings pane and the thread side panel.
 *
 * It is deliberately just an image + input controls: the page itself lives in
 * an external headless Chromium, and every interaction goes through CDP.
 */
export function BrowserViewport({
  live,
  fill = false,
  hideToolbar = false,
}: BrowserViewportProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      {!hideToolbar && (
        <Row className="gap-1.5">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void live.refresh()}
            title={t("画面を更新")}
          >
            <RefreshCw size={12} /> {t("更新")}
          </Button>
          <Button
            size="sm"
            selected={live.live}
            onClick={() => live.setLive((value) => !value)}
          >
            {live.live ? <Square size={12} /> : <Play size={12} />}
            {live.live ? t("停止") : t("ライブ表示")}
          </Button>
        </Row>
      )}
      {live.live ? (
        <>
          <div
            className={
              "flex items-center justify-center overflow-auto rounded-md border border-line bg-inset p-1 " +
              (fill ? "min-h-0 flex-1" : "min-h-48")
            }
          >
            {live.frameSrc ? (
              <button
                type="button"
                className="block cursor-crosshair border-0 bg-transparent p-0"
                aria-label={t("ブラウザのライブ画面をクリックして操作")}
                onClick={(event) => {
                  const image = event.currentTarget.querySelector("img");
                  if (!image) return;
                  const point = framePoint(
                    event.clientX,
                    event.clientY,
                    image.getBoundingClientRect(),
                    {
                      width: image.naturalWidth,
                      height: image.naturalHeight,
                    },
                    live.frameSize,
                  );
                  if (point) void live.clickAt(point.x, point.y);
                }}
              >
                <img
                  src={live.frameSrc}
                  alt={t("ブラウザのライブ画面")}
                  className={
                    fill
                      ? "max-h-full max-w-full object-contain"
                      : "max-h-[28rem] max-w-full object-contain"
                  }
                />
              </button>
            ) : (
              <Spinner size={13} label={t("画面を取得しています…")} />
            )}
          </div>
          {live.frameError && (
            <p className="m-0 text-xs text-danger">{live.frameError}</p>
          )}
          <p className="m-0 text-xs text-muted">
            {t(
              "画面をクリックするとその位置を操作します。入力欄をクリックしてから下の欄に入力してください。",
            )}
          </p>
          <Row className="gap-1.5">
            <Input
              className="min-w-0 flex-1"
              aria-label={t("ブラウザへの入力")}
              placeholder={t("フォーカス中の欄に入力")}
              value={live.liveText}
              onChange={(event) => live.setLiveText(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  const value = live.liveText;
                  live.setLiveText("");
                  void live.typeActive(value);
                } else if (event.key === "Escape") {
                  // Esc belongs to the page while this field is focused; do
                  // not let the app shortcut close the browser pane.
                  event.stopPropagation();
                }
              }}
            />
            <Button
              size="sm"
              disabled={!live.liveText}
              onClick={() => {
                const value = live.liveText;
                live.setLiveText("");
                void live.typeActive(value);
              }}
            >
              {t("入力する")}
            </Button>
            {["Enter", "Tab", "Escape"].map((name) => (
              <Button
                key={name}
                size="sm"
                onClick={() => void live.pressKey(name)}
              >
                {name}
              </Button>
            ))}
          </Row>
        </>
      ) : (
        <div className="flex min-h-24 items-center justify-center rounded-md border border-dashed border-line bg-inset/40 p-4 text-center text-xs text-muted">
          {t("ライブ表示を開始すると、ブラウザ画面をここで確認できます。")}
        </div>
      )}
    </div>
  );
}
