import type { SendShortcut } from "@/lib/ui";
import type { ModelView, ProjectView, ProviderView } from "@/lib/types";

export interface DraftAttachment {
  kind: string;
  mime_type: string;
  data_base64: string;
  filename: string;
}

export type ComposerMode = "chat" | "thinking" | "agent";

export interface ComposerProps {
  /** Direct-from-store defaults apply when omitted: ChatPane renders
   *  `<Composer onSend onStop busy draftKey />` inside a
   *  `ChatComposerProvider` and no longer drills these. Explicit props still
   *  win (tests, previews). */
  mode?: ComposerMode;
  setMode?: (mode: ComposerMode) => void;
  /** Modes the active surface exposes. When empty, fall back to ALL_MODES. */
  allowedModes?: ReadonlyArray<ComposerMode>;
  models?: ModelView[];
  modelId?: string;
  setModelId?: (id: string) => void;
  /** Providers, for resolving a model's provider name in the picker. */
  providers?: ProviderView[];
  projects?: ProjectView[];
  projectId?: string | null;
  onProjectChange?: (id: string | null) => void;
  onCreateProject?: (name: string) => Promise<string | null>;
  /** While true the model/project lists are "not known yet", not "empty". The
   *  menus used to say "No model configured — add a provider first" on every
   *  open that happened before the query settled. */
  modelsPending?: boolean;
  projectsPending?: boolean;
  busy: boolean;
  onSend: (
    content: string,
    attachments: DraftAttachment[],
    options: { temporaryMode: boolean; allowTools: boolean },
  ) => void | Promise<unknown>;
  onStop: () => void;
  /** Sends text to a Run that is already working, instead of stopping it. */
  onSteer?: (content: string) => void | Promise<unknown>;
  /** Key shortcut used to send messages (Enter vs Cmd/Ctrl+Enter). */
  sendShortcut?: SendShortcut;
  /** Drafts are scoped per conversation so switching threads no longer drops
   *  (or leaks) what the user was typing. `null` is the "no thread yet" slot. */
  draftKey?: string | null;
}
