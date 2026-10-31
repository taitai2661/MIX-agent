/// Image attachments for the composer draft.
///
/// Covers paste/drop/file-picker intake, the 8 MB per-file guard and the
/// `MAX_ATTACHMENTS` cap. Vision-compatibility warnings stay in `Composer`,
/// which owns the selected model.

import { useRef, useState } from "react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import type { DraftAttachment } from "./composerTypes";

export const MAX_ATTACHMENTS = 8;
const MAX_FILE_BYTES = 8 * 1024 * 1024;

export function useAttachments(): {
  attachments: DraftAttachment[];
  attachError: string | null;
  attaching: boolean;
  addImageFiles: (files: Iterable<File>) => Promise<void>;
  onAttach: () => Promise<void>;
  removeAttachment: (index: number) => void;
  clearAttachments: () => void;
} {
  const [attachments, setAttachments] = useState<DraftAttachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attaching, setAttaching] = useState(false);
  const attachingRef = useRef(false);

  const pushCapped = (item: DraftAttachment) => {
    setAttachments((prev) =>
      prev.length >= MAX_ATTACHMENTS ? prev : [...prev, item],
    );
  };

  const addImageFiles = async (files: Iterable<File>) => {
    if (attachingRef.current) return;
    const incoming = [...files].filter((f) => f.type.startsWith("image/"));
    if (incoming.length === 0) return;
    if (attachments.length >= MAX_ATTACHMENTS) {
      setAttachError(tf("添付は最大{max}件までです", { max: MAX_ATTACHMENTS }));
      return;
    }
    attachingRef.current = true;
    setAttachError(null);
    setAttaching(true);
    try {
      for (const file of incoming) {
        if (file.size > MAX_FILE_BYTES) {
          setAttachError(
            `${file.name}: ${t("添付ファイルが大きすぎます (上限8MB)")}`,
          );
          continue;
        }
        try {
          const dataUrl = await new Promise<string>((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(String(reader.result));
            reader.onerror = () => reject(new Error("read failed"));
            reader.readAsDataURL(file);
          });
          const comma = dataUrl.indexOf(",");
          pushCapped({
            kind: "image",
            mime_type: file.type || "image/png",
            data_base64: comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl,
            filename: file.name || "pasted-image.png",
          });
        } catch (e) {
          setAttachError(errorMessage(e));
        }
      }
    } finally {
      attachingRef.current = false;
      setAttaching(false);
    }
  };

  const onAttach = async () => {
    if (attachingRef.current) return;
    setAttachError(null);
    attachingRef.current = true;
    setAttaching(true);
    try {
      const paths = await api.pickOpenFile({
        multiple: true,
        filters: [
          {
            name: "images",
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp"],
          },
        ],
      });
      for (const path of paths) {
        try {
          const payload = await api.readAttachment(path);
          pushCapped({
            kind: payload.kind,
            mime_type: payload.mime_type,
            data_base64: payload.data_base64,
            filename: payload.filename,
          });
        } catch (e) {
          setAttachError(errorMessage(e));
        }
      }
    } finally {
      attachingRef.current = false;
      setAttaching(false);
    }
  };

  const removeAttachment = (index: number) => {
    setAttachments((prev) => prev.filter((_, j) => j !== index));
  };

  const clearAttachments = () => setAttachments([]);

  return {
    attachments,
    attachError,
    attaching,
    addImageFiles,
    onAttach,
    removeAttachment,
    clearAttachments,
  };
}
