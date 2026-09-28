import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { useUi } from "@/lib/ui";
import type { ProjectView } from "@/lib/types";

/** The project currently driving the file tree / diff scope. */
export function useActiveProject(): ProjectView | null {
  const activeProjectId = useUi((s) => s.activeProjectId);
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: api.listProjects,
    staleTime: 30_000,
  });
  if (!activeProjectId) return null;
  return projects.data?.find((p) => p.id === activeProjectId) ?? null;
}

/** Absolute folder of the active project (`null` when unset / missing). */
export function useActiveRoot(): string | null {
  return useActiveProject()?.rootPath ?? null;
}
