/// Attachment chips shown above the textarea plus the drop-target overlay.

import { AlertTriangle, Paperclip, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import type { DraftAttachment } from "./composerTypes";
import { MAX_ATTACHMENTS } from "./useAttachments";

interface ComposerAttachmentsProps {
  attachments: DraftAttachment[];
  /** An explicitly picked model without vision is paired with an image. */
  visionWarning: boolean;
  onRemove: (index: number) => void;
}

export function ComposerAttachments({
  attachments,
  visionWarning,
  onRemove,
}: ComposerAttachmentsProps) {
  if (attachments.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5 border-b border-line px-3 py-2">
      {attachments.map((a, i) => (
        <span
          key={`${a.filename}-${i}`}
          className={cn(
            "inline-flex items-center gap-1 rounded-md border border-line bg-inset px-1.5 py-0.5 text-xs text-muted",
            "transition-colors hover:border-line-strong",
          )}
        >
          {a.mime_type.startsWith("image/") ? (
            <img
              src={`data:${a.mime_type};base64,${a.data_base64}`}
              alt={a.filename}
              className="size-5 rounded-sm object-cover"
            />
          ) : (
            <Paperclip size={12} className="shrink-0" />
          )}
          <span className="max-w-40 truncate">{a.filename}</span>
          {a.mime_type.startsWith("image/") && visionWarning && (
            <AlertTriangle
              size={12}
              role="img"
              aria-label={t("このモデルは画像入力に対応していません")}
              className="text-warning"
            />
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => onRemove(i)}
            aria-label={`${a.filename} ${t("削除")}`}
          >
            <X size={13} />
          </Button>
        </span>
      ))}
      <span className="self-center text-xs text-faint">
        {attachments.length} / {MAX_ATTACHMENTS}
      </span>
    </div>
  );
}

export function ComposerDropOverlay() {
  return (
    <div className="pointer-events-none absolute inset-0 z-30 flex flex-col items-center justify-center rounded-2xl border-2 border-dashed border-brand bg-surface/90 backdrop-blur-xs">
      <Paperclip size={22} className="text-brand animate-bounce" />
      <span className="mt-1 text-xs font-semibold text-brand">
        {t("ファイルをドロップして添付")}
      </span>
    </div>
  );
}
