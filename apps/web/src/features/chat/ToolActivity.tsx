import { t } from "@/app/i18n";
import { Brain, FileText, Globe2, Search, Terminal, Wrench } from "lucide-react";

export type ActivitySummary = {
  icon?: string;
  label: string;
  detail?: string;
  sources?: { host: string; url: string }[];
  remaining?: number;
};

export function ActivityIcon({ icon, size = 22 }: { icon?: string; size?: number }) {
  const Icon =
    icon === "search" ? Search :
    icon === "globe" ? Globe2 :
    icon === "file" ? FileText :
    icon === "terminal" ? Terminal :
    icon === "memory" || icon === "plan" ? Brain : Wrench;
  return <Icon aria-hidden="true" size={size} />;
}

export function ToolActivity({ activity, running = false }: { activity: ActivitySummary; running?: boolean }) {
  return (
    <section className={"tool-activity" + (running ? " running" : "")} aria-live="polite">
      <div className="tool-activity-title">
        <ActivityIcon icon={activity.icon} />
        <span>{activity.detail ? `${activity.detail} を検索中` : activity.label}</span>
        {!running && <span className="tool-activity-chevron">⌄</span>}
      </div>
      {!running && !!activity.sources?.length && (
        <div className="tool-activity-sources">
          {activity.sources.map((source) => (
            <a key={source.url} href={source.url} target="_blank" rel="noreferrer">
              <Globe2 aria-hidden="true" size={14} />
              {source.host}
            </a>
          ))}
          {!!activity.remaining && <span>{t("あと")} {activity.remaining} {t("件")}</span>}
        </div>
      )}
    </section>
  );
}
