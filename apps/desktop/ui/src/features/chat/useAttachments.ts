/// Image attachments for the composer draft.
///
/// Covers paste/drop/file-picker intake, the 8 MB per-file guard and the
/// `MAX_ATTACHMENTS` cap, plus one-shot screen capture via
/// `getDisplayMedia` (the Tauri shell has no native screenshot command, so
/// the WebView captures a shared screen/window and keeps a single frame).
/// Vision-compatibility warnings stay in `Composer`, which owns the selected
/// model.

import { useRef, useState } from "react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import type { DraftAttachment } from "./composerTypes";

export const MAX_ATTACHMENTS = 8;
const MAX_FILE_BYTES = 8 * 1024 * 1024;

function timestampedName(prefix: string, ext: string): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${prefix}-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}.${ext}`;
}

function extForMime(mime: string): string {
  if (mime === "image/jpeg") return "jpg";
  if (mime === "image/gif") return "gif";
  if (mime === "image/webp") return "webp";
  if (mime === "image/bmp") return "bmp";
  return "png";
}

export function screenshotSupported(): boolean {
  return (
    typeof navigator !== "undefined" &&
    !!navigator.mediaDevices &&
    typeof navigator.mediaDevices.getDisplayMedia === "function"
  );
}

export function useAttachments(): {
  attachments: DraftAttachment[];
  attachError: string | null;
  attaching: boolean;
  addImageFiles: (files: Iterable<File>) => Promise<void>;
  addImageBlob: (blob: Blob, filename?: string) => Promise<void>;
  onAttach: () => Promise<void>;
  captureScreenshot: () => Promise<boolean>;
  screenshotAvailable: boolean;
  removeAttachment: (index: number) => void;
  /** Drop only the attachments that were actually sent (by identity), so an
   *  image added while the request is in flight stays in the composer. */
  removeAttachments: (items: readonly DraftAttachment[]) => void;
  clearAttachments: () => void;
} {
  const [attachments, setAttachments] = useState<DraftAttachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attaching, setAttaching] = useState(false);
  const attachingRef = useRef(false);
  // Count is kept in a ref as well as in state: the async intake paths need to
  // know the remaining slots *now*, not the value captured by the current
  // render's closure (two batches in flight would otherwise both see room).
  const countRef = useRef(0);

  /** Returns false (instead of silently dropping) once the cap is reached. */
  const pushCapped = (item: DraftAttachment): boolean => {
    if (countRef.current >= MAX_ATTACHMENTS) return false;
    countRef.current += 1;
    setAttachments((prev) => [...prev, item]);
    return true;
  };

  const fileToAttachment = async (file: Blob, fallbackName: string) => {
    const mime = file.type || "image/png";
    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(new Error("read failed"));
      reader.readAsDataURL(file);
    });
    const comma = dataUrl.indexOf(",");
    pushCapped({
      kind: "image",
      mime_type: mime,
      data_base64: comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl,
      filename: fallbackName,
    });
  };

  const capError = () =>
    tf("添付は最大{max}件までです", { max: MAX_ATTACHMENTS });

  const addImageFiles = async (files: Iterable<File>) => {
    if (attachingRef.current) return;
    const incoming = [...files].filter((f) => f.type.startsWith("image/"));
    if (incoming.length === 0) return;
    const remaining = MAX_ATTACHMENTS - countRef.current;
    if (remaining <= 0) {
      setAttachError(capError());
      return;
    }
    const accepted = incoming.slice(0, remaining);
    const droppedByCap = incoming.length - accepted.length;
    attachingRef.current = true;
    setAttachError(null);
    setAttaching(true);
    const errors: string[] = [];
    try {
      for (const file of accepted) {
        if (file.size > MAX_FILE_BYTES) {
          errors.push(
            `${file.name}: ${t("添付ファイルが大きすぎます (上限8MB)")}`,
          );
          continue;
        }
        try {
          await fileToAttachment(
            file,
            file.name || timestampedName("pasted-image", extForMime(file.type)),
          );
        } catch (e) {
          errors.push(errorMessage(e));
        }
      }
      if (droppedByCap > 0) errors.push(capError());
      if (errors.length) setAttachError(errors.join("\n"));
    } finally {
      attachingRef.current = false;
      setAttaching(false);
    }
  };

  const addImageBlob = async (blob: Blob, filename?: string) => {
    await addImageFiles([
      new File(
        [blob],
        filename ?? timestampedName("pasted-image", extForMime(blob.type)),
        { type: blob.type || "image/png" },
      ),
    ]);
  };

  const captureScreenshot = async (): Promise<boolean> => {
    if (attachingRef.current) return false;
    if (!screenshotSupported()) {
      setAttachError(t("この環境では画面キャプチャに対応していません"));
      return false;
    }
    if (countRef.current >= MAX_ATTACHMENTS) {
      setAttachError(capError());
      return false;
    }
    let stream: MediaStream | null = null;
    let video: HTMLVideoElement | null = null;
    try {
      attachingRef.current = true;
      setAttachError(null);
      setAttaching(true);
      stream = await navigator.mediaDevices.getDisplayMedia({
        video: true,
        audio: false,
      });
      const track = stream.getVideoTracks()[0];
      if (!track) {
        setAttachError(t("画面キャプチャを開始できませんでした"));
        return false;
      }
      const frame: unknown = await (
        track as unknown as {
          requestFrame?: () => Promise<unknown>;
        }
      ).requestFrame?.();
      void frame;
      video = document.createElement("video");
      const element = video;
      element.muted = true;
      element.srcObject = new MediaStream([track]);
      await element.play();
      await new Promise<void>((resolve) => {
        if (element.readyState >= 2 && element.videoWidth > 0) {
          resolve();
          return;
        }
        const timer = window.setTimeout(() => resolve(), 1500);
        element.onloadeddata = () => {
          window.clearTimeout(timer);
          resolve();
        };
      });
      const width = element.videoWidth || 1280;
      const height = element.videoHeight || 800;
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const ctx2d = canvas.getContext("2d");
      if (!ctx2d) {
        setAttachError(t("画面キャプチャを開始できませんでした"));
        return false;
      }
      ctx2d.drawImage(element, 0, 0, width, height);
      element.pause();
      const blob = await new Promise<Blob | null>((resolve) =>
        canvas.toBlob((b) => resolve(b), "image/png"),
      );
      if (!blob) {
        setAttachError(t("画面キャプチャを開始できませんでした"));
        return false;
      }
      if (blob.size > MAX_FILE_BYTES) {
        setAttachError(t("添付ファイルが大きすぎます (上限8MB)"));
        return false;
      }
      await fileToAttachment(blob, timestampedName("screenshot", "png"));
      return true;
    } catch (e) {
      // The picker dialog throws AbortError/NotAllowedError when the user
      // cancels: stay silent instead of painting a red error box.
      const name =
        e instanceof DOMException ? e.name : e instanceof Error ? e.name : "";
      if (name === "AbortError" || name === "NotAllowedError") return false;
      setAttachError(errorMessage(e));
      return false;
    } finally {
      // Stopping the track alone leaves the <video> holding a dead MediaStream;
      // release it explicitly so the capture indicator and decoder are freed.
      if (video) {
        video.pause();
        video.srcObject = null;
      }
      stream?.getTracks().forEach((tr) => {
        try {
          tr.stop();
        } catch {
          /* already stopped */
        }
      });
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
      const errors: string[] = [];
      let dropped = 0;
      for (const path of paths) {
        try {
          const payload = await api.readAttachment(path);
          if (
            !pushCapped({
              kind: payload.kind,
              mime_type: payload.mime_type,
              data_base64: payload.data_base64,
              filename: payload.filename,
            })
          )
            dropped += 1;
        } catch (e) {
          errors.push(errorMessage(e));
        }
      }
      if (dropped > 0) errors.push(capError());
      if (errors.length) setAttachError(errors.join("\n"));
    } finally {
      attachingRef.current = false;
      setAttaching(false);
    }
  };

  const removeAttachment = (index: number) => {
    if (index < 0 || index >= countRef.current) return;
    countRef.current -= 1;
    setAttachments((prev) => prev.filter((_, j) => j !== index));
  };

  const removeAttachments = (items: readonly DraftAttachment[]) => {
    if (items.length === 0) return;
    const sent = new Set(items);
    const removed = attachments.filter((item) => sent.has(item)).length;
    if (removed === 0) return;
    countRef.current = Math.max(0, countRef.current - removed);
    setAttachments((prev) => prev.filter((item) => !sent.has(item)));
  };

  const clearAttachments = () => {
    countRef.current = 0;
    setAttachments([]);
  };

  return {
    attachments,
    attachError,
    attaching,
    addImageFiles,
    addImageBlob,
    onAttach,
    captureScreenshot,
    screenshotAvailable: screenshotSupported(),
    removeAttachment,
    removeAttachments,
    clearAttachments,
  };
}
