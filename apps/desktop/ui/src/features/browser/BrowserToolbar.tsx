import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Globe,
  Lock,
  Maximize2,
  Minimize2,
  Pin,
  PinOff,
  Play,
  RefreshCw,
  RotateCw,
  Settings2,
  Square,
  TriangleAlert,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import type { LiveBrowser } from "./BrowserViewport";

/** Turn whatever the user typed into a URL the egress gate can evaluate. */
export function normalizeAddress(raw: string): string {
  const value = raw.trim();
  if (!value) return "";
  // Only treat a real scheme delimiter as a scheme: `localhost:3000` looks
  // superficially like `scheme:`, but it is a host:port.
  if (/^(https?:\/\/|about:)/i.test(value)) return value;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(value)) return value;
  if (
    /^(localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0)(:\d+)?(\/|$)/i.test(value)
  ) {
    return `http://${value}`;
  }
  return `https://${value}`;
}

/** Host (+ path) shown when the address bar is not being edited. */
export function displayAddress(raw: string | null): string {
  if (!raw || raw === "about:blank") return raw ?? "";
  try {
    const url = new URL(raw);
    const path = url.pathname === "/" ? "" : url.pathname;
    return `${url.host}${path}${url.search}`;
  } catch {
    return raw;
  }
}

interface BrowserToolbarProps {
  live: LiveBrowser;
  agentBusy: boolean;
  autoOpen: boolean;
  full: boolean;
  autoFocusAddress: boolean;
  onToggleAutoOpen: () => void;
  onToggleFull: () => void;
  onOpenSettings: () => void;
  onClose: () => void;
}

export function BrowserToolbar({
  live,
  agentBusy,
  autoOpen,
  full,
  autoFocusAddress,
  onToggleAutoOpen,
  onToggleFull,
  onOpenSettings,
  onClose,
}: BrowserToolbarProps) {
  const [address, setAddress] = useState("");
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const currentUrl = live.page.url ?? "";
  useEffect(() => {
    if (!editing) setAddress(displayAddress(live.page.url));
  }, [editing, live.page.url]);

  useEffect(() => {
    if (!autoFocusAddress) return;
    if (live.page.url && live.page.url !== "about:blank") return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [autoFocusAddress, live.page.url]);

  // ⌘L / the command palette can ask for the address bar without knowing
  // where the input lives.
  useEffect(() => {
    const focus = () => {
      setEditing(true);
      setAddress(currentUrl);
      inputRef.current?.focus();
      inputRef.current?.select();
    };
    window.addEventListener("mix:browser-focus-address", focus);
    return () => window.removeEventListener("mix:browser-focus-address", focus);
  }, [currentUrl]);

  const security = currentUrl.startsWith("https:")
    ? { icon: Lock, className: "text-success" }
    : currentUrl.startsWith("http:")
      ? { icon: TriangleAlert, className: "text-warning" }
      : { icon: Globe, className: "text-faint" };
  const SecurityIcon = security.icon;

  const withBusy = async (task: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await task();
    } finally {
      window.setTimeout(() => setBusy(false), 600);
    }
  };

  const submit = async () => {
    const url = normalizeAddress(address);
    if (!url) return;
    setEditing(false);
    inputRef.current?.blur();
    await withBusy(() => live.navigate(url));
  };

  return (
    <div className="flex h-9 shrink-0 items-center gap-1 border-b border-line bg-bg/85 px-2 backdrop-blur-md">
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => void withBusy(() => live.back())}
        disabled={!live.page.canGoBack}
        title={t("戻る")}
        aria-label={t("戻る")}
      >
        <ArrowLeft size={13} />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => void withBusy(() => live.forward())}
        disabled={!live.page.canGoForward}
        title={t("進む")}
        aria-label={t("進む")}
      >
        <ArrowRight size={13} />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() =>
          void withBusy(() => (busy ? live.stop() : live.reload()))
        }
        title={busy ? t("停止") : t("再読み込み")}
        aria-label={busy ? t("停止") : t("再読み込み")}
      >
        {busy ? <X size={13} /> : <RefreshCw size={13} />}
      </Button>

      <div
        className={cn(
          "group flex h-7 min-w-0 flex-1 items-center gap-1 rounded-lg border px-2 transition-colors",
          editing
            ? "border-line-strong bg-surface"
            : "border-transparent bg-inset hover:border-line",
        )}
      >
        <SecurityIcon
          size={12}
          className={cn("shrink-0", security.className)}
          aria-hidden="true"
        />
        <input
          ref={inputRef}
          value={address}
          onChange={(event) => setAddress(event.target.value)}
          onFocus={(event) => {
            setEditing(true);
            setAddress(currentUrl);
            event.currentTarget.select();
          }}
          onBlur={() => {
            setEditing(false);
            setAddress(displayAddress(live.page.url));
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void submit();
            } else if (event.key === "Escape") {
              event.preventDefault();
              // Keep the app-level "Esc closes the side panel" shortcut from
              // firing while the user is cancelling address editing.
              event.stopPropagation();
              setEditing(false);
              setAddress(displayAddress(live.page.url));
              event.currentTarget.blur();
            }
          }}
          aria-label={t("アドレスバー")}
          placeholder={t("URL または localhost:3000")}
          spellCheck={false}
          className="min-w-0 flex-1 bg-transparent font-mono text-xs text-fg outline-none placeholder:text-faint"
        />
        {currentUrl && (
          <button
            type="button"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              const url = normalizeAddress(editing ? address : currentUrl);
              if (url)
                void api
                  .openExternal(url)
                  .catch((e) => console.warn(errorMessage(e)));
            }}
            title={t("システムのブラウザで開く")}
            aria-label={t("システムのブラウザで開く")}
            className="shrink-0 rounded p-0.5 text-faint opacity-0 transition-opacity hover:text-fg group-hover:opacity-100 focus-visible:opacity-100"
          >
            <ExternalLink size={12} />
          </button>
        )}
      </div>

      {agentBusy && (
        <span className="hidden shrink-0 items-center gap-1 rounded-full bg-brand-soft px-2 py-0.5 text-2xs text-brand sm:flex">
          <RotateCw size={10} className="animate-spin" />
          {t("エージェント操作中")}
        </span>
      )}

      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => live.setLive((value) => !value)}
        title={live.live ? t("ライブ表示を停止") : t("ライブ表示を開始")}
        aria-label={live.live ? t("ライブ表示を停止") : t("ライブ表示を開始")}
        aria-pressed={live.live}
      >
        {live.live ? <Square size={12} /> : <Play size={12} />}
      </Button>

      <Button
        variant="ghost"
        size="icon-sm"
        onClick={onToggleAutoOpen}
        title={
          autoOpen
            ? t("エージェント使用時の自動表示をオフ")
            : t("エージェント使用時に自動表示")
        }
        aria-label={
          autoOpen
            ? t("エージェント使用時の自動表示をオフ")
            : t("エージェント使用時に自動表示")
        }
        aria-pressed={autoOpen}
      >
        {autoOpen ? <Pin size={13} /> : <PinOff size={13} />}
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={onToggleFull}
        title={full ? t("スプリット表示に戻す") : t("フル表示")}
        aria-label={full ? t("スプリット表示に戻す") : t("フル表示")}
      >
        {full ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={onOpenSettings}
        title={t("ブラウザ設定を開く")}
        aria-label={t("ブラウザ設定を開く")}
      >
        <Settings2 size={13} />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={onClose}
        title={t("閉じる")}
        aria-label={t("閉じる")}
      >
        <X size={14} />
      </Button>
    </div>
  );
}
