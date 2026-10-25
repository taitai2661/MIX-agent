/// Composer state provider: ends the ChatPane → Composer → ModelPicker drilling.
///
/// ChatPane used to thread 14 props (mode, modelId, projectId, the three
/// lists, their pending flags and three callbacks) into `Composer`, which
/// forwarded a subset into `ModelPicker`. Every store flush re-created the
/// inline arrows and defeated the `memo` on both. The state itself already
/// lives in global stores — `useUi` for the picks, react-query for the lists —
/// so this provider reads it at the consumption site via selectors and
/// exposes stable callbacks. `Composer` props stay accepted as overrides (the
/// settings pane reuses `ModelPicker` directly, and tests render `Composer`
/// without a provider), but ChatPane no longer passes them.

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import {
  CONVERSATIONS_QUERY_KEY,
  PROJECTS_QUERY_KEY,
  useModels,
  useProviders,
  useProjects,
} from "@/lib/queries";
import { SURFACE_MODES, useUi, type RunMode } from "@/lib/ui";
import { setLastModelForSurface } from "@/lib/modelPrefs";
import type { ModelView, ProjectView, ProviderView } from "@/lib/types";

/** Shared frozen empties, so `memo` is not defeated by a fresh `[]`. */
export const SHARED_EMPTY_MODELS: ModelView[] = [];
export const SHARED_EMPTY_PROJECTS: ProjectView[] = [];
export const SHARED_EMPTY_PROVIDERS: ProviderView[] = [];

export interface ChatComposerState {
  mode: RunMode;
  setMode: (mode: RunMode) => void;
  allowedModes: readonly RunMode[];
  models: ModelView[];
  modelId: string;
  setModelId: (id: string) => void;
  providers: ProviderView[];
  projects: ProjectView[];
  projectId: string | null;
  onProjectChange: (id: string | null) => void;
  onCreateProject: (name: string) => Promise<string | null>;
  modelsPending: boolean;
  projectsPending: boolean;
  /** Last project-switch failure, surfaced by ChatPane next to send errors. */
  projectError: string | null;
}

const ChatComposerContext = createContext<ChatComposerState | null>(null);

/** `Composer`'s fallback when rendered without a provider (tests). */
export function useChatComposerContext(): ChatComposerState | null {
  return useContext(ChatComposerContext);
}

export function ChatComposerProvider({ children }: { children: ReactNode }) {
  const mode = useUi((s) => s.mode);
  const setMode = useUi((s) => s.setMode);
  const modelId = useUi((s) => s.modelId);
  const setModelIdRaw = useUi((s) => s.setModelId);
  const surface = useUi((s) => s.surface);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const setConversationId = useUi((s) => s.setConversationId);
  const queryClient = useQueryClient();
  const models = useModels();
  const providers = useProviders();
  const projects = useProjects();
  const [projectError, setProjectError] = useState<string | null>(null);

  const allowedModes = SURFACE_MODES[surface];

  // Remember the pick per surface so the next launch starts on the model the
  // user actually chose instead of resetting to the first row.
  const setModelId = useCallback(
    (id: string) => {
      setModelIdRaw(id);
      setLastModelForSurface(useUi.getState().surface, id);
    },
    [setModelIdRaw],
  );

  const selectProject = useCallback(
    async (projectId: string | null) => {
      const prev = useUi.getState().activeProjectId;
      if (projectId === prev) return;
      setProjectError(null);
      setActiveProjectId(projectId);
      try {
        const created = await api.createConversation({
          mode: useUi.getState().mode,
          model_id: useUi.getState().modelId || "auto",
          title: null,
          workspace_id: null,
          project_id: projectId,
        });
        setConversationId(created.id);
        void queryClient.invalidateQueries({
          queryKey: CONVERSATIONS_QUERY_KEY,
        });
      } catch (error) {
        setActiveProjectId(prev);
        setProjectError(errorMessage(error));
      }
    },
    [queryClient, setActiveProjectId, setConversationId],
  );

  const createProject = useCallback(
    async (name: string) => {
      // Folder-first: every project is rooted at a directory, so pick one
      // before creating. Cancelling the picker aborts and returns null.
      const path = await openDialog({ directory: true, multiple: false });
      if (typeof path !== "string" || !path) return null;
      const created = await api.createProject({
        name: name.trim(),
        instructions: "",
        rootPath: path,
        sources: [],
      });
      void queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      return created.id;
    },
    [queryClient],
  );

  const value = useMemo<ChatComposerState>(
    () => ({
      mode,
      setMode,
      allowedModes,
      models: models.data ?? SHARED_EMPTY_MODELS,
      modelId: modelId || "auto",
      setModelId,
      providers: providers.data ?? SHARED_EMPTY_PROVIDERS,
      projects: projects.data ?? SHARED_EMPTY_PROJECTS,
      projectId: activeProjectId,
      onProjectChange: (id) => void selectProject(id),
      onCreateProject: createProject,
      modelsPending: models.isPending,
      projectsPending: projects.isPending,
      projectError,
    }),
    [
      mode,
      setMode,
      allowedModes,
      models.data,
      models.isPending,
      modelId,
      setModelId,
      providers.data,
      projects.data,
      projects.isPending,
      activeProjectId,
      selectProject,
      createProject,
      projectError,
    ],
  );

  return (
    <ChatComposerContext.Provider value={value}>
      {children}
    </ChatComposerContext.Provider>
  );
}
