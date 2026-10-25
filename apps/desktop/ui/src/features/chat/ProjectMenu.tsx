/// Project picker + inline project creation for the composer toolbar.
///
/// Always rendered (the picker keeps its slot at every width so the toolbar
/// never reflows); long names truncate inside.

import { useEffect, useRef, useState } from "react";
import { Check, Folder, Plus } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { errorMessage } from "@/lib/errors";
import type { ProjectView } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  DropdownMenuTriggerButton,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { ErrorBadge } from "@/components/shared";

export function ProjectMenu({
  projects,
  projectId = null,
  onProjectChange,
  onCreateProject,
  pending = false,
}: {
  projects: ProjectView[];
  projectId?: string | null;
  onProjectChange?: (id: string | null) => void;
  onCreateProject?: (name: string) => Promise<string | null>;
  /** While true the project list is "not known yet", not "empty". */
  pending?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [menuMode, setMenuMode] = useState<"list" | "create">("list");
  const [draftName, setDraftName] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement | null>(null);

  const activeProject = projects.find((p) => p.id === projectId) ?? null;

  const submitNewProject = async () => {
    const name = draftName.trim();
    if (!name || creating || !onCreateProject) return;
    setCreating(true);
    setCreateError(null);
    try {
      const created = await onCreateProject(name);
      // `null` means the user backed out (cancelled the folder picker):
      // keep the form open so the draft name is not thrown away.
      if (!created) return;
      setDraftName("");
      setCreateError(null);
      setMenuMode("list");
      setOpen(false);
    } catch (e) {
      setCreateError(errorMessage(e));
    } finally {
      setCreating(false);
    }
  };

  const cancelNewProject = () => {
    setDraftName("");
    setCreateError(null);
    setMenuMode("list");
  };

  const resetMenu = (next: boolean) => {
    setOpen(next);
    if (!next) {
      setMenuMode("list");
      setDraftName("");
      setCreateError(null);
    }
  };

  useEffect(() => {
    if (menuMode === "create") {
      const id = window.setTimeout(() => nameRef.current?.focus(), 0);
      return () => window.clearTimeout(id);
    }
    return undefined;
  }, [menuMode]);

  return (
    <span className="contents">
      <DropdownMenu open={open} onOpenChange={resetMenu}>
        <DropdownMenuTrigger asChild>
          <DropdownMenuTriggerButton
            aria-label={t("プロジェクトを選択")}
            title={activeProject?.name ?? t("プロジェクトなし")}
            className={cn(
              "max-w-32 min-w-0 shrink rounded-full border-transparent bg-transparent hover:bg-hover sm:max-w-48",
              !activeProject && !pending && "text-faint",
            )}
          >
            <Folder
              size={13}
              className="shrink-0 text-muted"
              aria-hidden="true"
            />
            <span className="min-w-0 flex-1 truncate text-start">
              {activeProject?.name ?? t("プロジェクトなし")}
            </span>
          </DropdownMenuTriggerButton>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className={cn("min-w-56", menuMode === "create" && "min-w-64")}
          onEscapeKeyDown={(e) => {
            if (menuMode === "create") {
              e.preventDefault();
              cancelNewProject();
            }
          }}
        >
          {menuMode === "list" ? (
            <>
              <DropdownMenuLabel>{t("プロジェクト")}</DropdownMenuLabel>
              <DropdownMenuItem onSelect={() => onProjectChange?.(null)}>
                <Folder size={13} className="text-muted" aria-hidden="true" />
                <span className="flex-1">{t("プロジェクトなし")}</span>
                {!projectId && (
                  <Check size={13} className="text-brand" aria-hidden="true" />
                )}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              {pending ? (
                <div className="flex items-center gap-1.5 px-2 py-1.5 text-xs text-muted">
                  <Spinner size={12} />
                  {t("読み込み中…")}
                </div>
              ) : (
                projects.length === 0 && (
                  <div className="px-2 py-1.5 text-xs text-muted">
                    {t("プロジェクトがありません")}
                  </div>
                )
              )}
              {projects.map((p) => (
                <DropdownMenuItem
                  key={p.id}
                  onSelect={() => onProjectChange?.(p.id)}
                >
                  <Folder size={13} className="text-muted" aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate">{p.name}</span>
                  {p.id === projectId && (
                    <Check
                      size={13}
                      className="text-brand"
                      aria-hidden="true"
                    />
                  )}
                </DropdownMenuItem>
              ))}
              {onCreateProject && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem
                    onSelect={(e) => {
                      e.preventDefault();
                      setMenuMode("create");
                    }}
                  >
                    <Plus size={13} className="text-muted" aria-hidden="true" />
                    <span className="flex-1">{t("新規プロジェクト")}…</span>
                  </DropdownMenuItem>
                </>
              )}
            </>
          ) : (
            <>
              <DropdownMenuLabel>{t("プロジェクトを作成")}</DropdownMenuLabel>
              <div className="px-1.5 pb-1.5">
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    void submitNewProject();
                  }}
                  className="flex flex-col gap-1.5"
                >
                  <Input
                    ref={nameRef}
                    placeholder={t("プロジェクト名")}
                    value={draftName}
                    onChange={(e) => setDraftName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        e.preventDefault();
                        e.stopPropagation();
                        cancelNewProject();
                      }
                    }}
                    disabled={creating}
                    maxLength={120}
                    aria-label={t("プロジェクト名")}
                  />
                  {createError && <ErrorBadge error={createError} />}
                  <div className="flex items-center justify-end gap-1.5 pt-0.5">
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={cancelNewProject}
                      disabled={creating}
                    >
                      {t("キャンセル")}
                    </Button>
                    <Button
                      type="submit"
                      variant="primary"
                      size="sm"
                      disabled={!draftName.trim() || creating}
                    >
                      {creating ? (
                        <Spinner size={12} />
                      ) : (
                        <Plus size={12} aria-hidden="true" />
                      )}
                      {t("作成")}
                    </Button>
                  </div>
                </form>
              </div>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </span>
  );
}
