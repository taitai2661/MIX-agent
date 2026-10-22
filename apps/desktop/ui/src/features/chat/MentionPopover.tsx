import { memo, useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileCode } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Spinner } from "@/components/ui/spinner";

interface MentionPopoverProps {
  rootPath: string | null;
  query: string;
  onSelect: (path: string) => void;
  onClose: () => void;
  selectedIndex: number;
  setSelectedIndex: (idx: number | ((prev: number) => number)) => void;
}

export const MentionPopover = memo(function MentionPopover({
  rootPath,
  query,
  onSelect,
  selectedIndex,
  setSelectedIndex,
}: MentionPopoverProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  const filesQuery = useQuery({
    queryKey: ["workspace_files_mention", rootPath, query],
    queryFn: () =>
      rootPath
        ? api.findWorkspaceFiles(rootPath, query, 20)
        : Promise.resolve([]),
    enabled: !!rootPath,
    staleTime: 5_000,
  });

  const files = filesQuery.data ?? [];

  useEffect(() => {
    // Reset selection index when query changes
    setSelectedIndex(0);
  }, [query, setSelectedIndex]);

  // Keep selected item visible in scroll
  useEffect(() => {
    if (!containerRef.current) return;
    const items = containerRef.current.querySelectorAll("[data-mention-item]");
    const active = items[selectedIndex] as HTMLElement | undefined;
    if (active) {
      active.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex]);

  if (!rootPath) return null;

  return (
    <div
      ref={containerRef}
      className="absolute bottom-full left-4 z-40 mb-2 max-h-60 w-80 overflow-y-auto rounded-xl border border-line bg-surface p-1 shadow-pop animate-scale"
      role="listbox"
      aria-label={t("ファイルメンション")}
    >
      <div className="flex items-center justify-between px-2.5 py-1.5 text-2xs font-medium text-faint border-b border-line">
        <span>{t("ファイルをメンション (@)")}</span>
        {filesQuery.isFetching && <Spinner size={10} />}
      </div>

      {files.length === 0 && !filesQuery.isFetching ? (
        <div className="px-3 py-2 text-xs text-faint">
          {t("一致するファイルがありません")}
        </div>
      ) : (
        <div className="flex flex-col gap-0.5 pt-1">
          {files.map((file, idx) => {
            const isSelected = idx === selectedIndex;
            const filename = file.split("/").pop() ?? file;
            const dirname = file.includes("/")
              ? file.slice(0, file.lastIndexOf("/"))
              : "";

            return (
              <button
                key={file}
                type="button"
                data-mention-item
                role="option"
                aria-selected={isSelected}
                onMouseEnter={() => setSelectedIndex(idx)}
                onClick={() => onSelect(file)}
                className={cn(
                  "flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-xs text-start transition-colors",
                  isSelected
                    ? "bg-hover text-fg font-medium"
                    : "text-muted hover:bg-hover hover:text-fg",
                )}
              >
                <FileCode size={13} className="shrink-0 text-brand" />
                <span className="truncate flex-1 font-mono">{filename}</span>
                {dirname && (
                  <span className="truncate max-w-28 text-2xs font-mono text-faint">
                    {dirname}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
});
