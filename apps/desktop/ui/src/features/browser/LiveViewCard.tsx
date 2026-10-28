import { Play, RefreshCw, Square } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Row } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { framePoint } from "./framePoint";
import type { useLiveFrame } from "./useLiveFrame";

type Live = ReturnType<typeof useLiveFrame>;

/** Live view and manual takeover. */
export function LiveViewCard({ live }: { live: Live }) {
  return (
    <Card
      title={t("ライブ画面")}
      icon={<Play size={13} />}
      action={
        <Row className="gap-1.5">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void live.refreshFrame()}
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
      }
    >
      {live.live ? (
        <>
          <div className="flex min-h-48 items-center justify-center overflow-auto rounded-md border border-line bg-inset p-1">
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
                  className="max-h-[28rem] max-w-full object-contain"
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
        <EmptyState
          title={t(
            "ライブ表示を開始すると、ブラウザ画面をここで確認できます。",
          )}
        />
      )}
    </Card>
  );
}
