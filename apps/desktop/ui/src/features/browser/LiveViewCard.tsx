import { Play, RefreshCw, Square } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Row } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { BrowserViewport } from "./BrowserViewport";
import type { useLiveFrame } from "./useLiveFrame";

type Live = ReturnType<typeof useLiveFrame>;

/** Live view and manual takeover inside the settings pane. */
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
      }
    >
      <BrowserViewport live={live} hideToolbar />
    </Card>
  );
}
