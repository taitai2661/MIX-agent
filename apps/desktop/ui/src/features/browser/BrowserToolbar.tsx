import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Globe,
  Lock,
  RefreshCw,
  RotateCw,
  TriangleAlert,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import { BrowserSessionMenu } from "./BrowserSessionMenu";
import type { WebviewBrowser } from "./useWebviewBrowser";

/** Turn whatever the user typed into a URL the egress gate can evaluate. */
export function normalizeAddress(raw: string): string {
  const value = raw.trim();
  if (!value) return "";
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
  browser: WebviewBrowser;
  workspaceId: string | null;
  agentBusy: boolean;
  autoOpen: boolean;
  full: boolean;
  docked: boolean;
  autoFocusAddress: boolean;
  onToggleAutoOpen: () => void;
  onToggleFull: () => void;
  onToggleDocked: () => void;
  onResetRatio: () => void;
  onClearData: () => void;
  onOpenSettings: () => void;
  onClose: () => void;
  onMenuOpenChange: (open: boolean) => void;
}

export function BrowserToolbar({
  browser,
  workspaceId,
  agentBusy,
  autoOpen,
  full,
  docked,
  autoFocusAddress,
  onToggleAutoOpen,
  onToggleFull,
  onToggleDocked,
  onResetRatio,
  onClearData,
  onOpenSettings,
  onClose,
  onMenuOpenChange,
}: BrowserToolbarProps) {
  const [address, setAddress] = useState("");
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const currentUrl = browser.page.url ?? "";
  const navigating = busy || browser.navigating;
  useEffect(() => {
    if (!editing) setAddress(displayAddress(browser.page.url));
  }, [editing, browser.page.url]);

  useEffect(() => {
    if (!autoFocusAddress) return;
    if (browser.page.url && browser.page.url !== "about:blank") return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [autoFocusAddress, browser.page.url]);

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
      window.setTimeout(() => setBusy(false), 400);
    }
  };

  const submit = async () => {
    const url = normalizeAddress(address);
    if (!url) return;
    setEditing(false);
    inputRef.current?.blur();
    await withBusy(() => browser.navigate(url));
  };

  return (
    <div className="flex h-10 shrink-0 items-center gap-1 border-b border-line bg-bg/90 px-2 backdrop-blur-md">
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => void withBusy(() => browser.back())}
        disabled={!browser.page.canGoBack}
        title={t("戻る")}
        aria-label={t("戻る")}
      >
        <ArrowLeft size={14} />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => void withBusy(() => browser.forward())}
        disabled={!browser.page.canGoForward}
        title={t("進む")}
        aria-label={t("進む")}
      >
        <ArrowRight size={14} />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() =>
          void withBusy(() => (navigating ? browser.stop() : browser.reload()))
        }
        title={navigating ? t("停止") : t("再読み込み")}
        aria-label={navigating ? t("停止") : t("再読み込み")}
      >
        {navigating ? <X size={14} /> : <RefreshCw size={14} />}
      </Button>

      <div
        className={cn(
          "group flex h-8 min-w-0 flex-1 items-center gap-1.5 rounded-full border px-3 transition-colors",
          editing
            ? "border-line-strong bg-surface"
            : "border-line/70 bg-inset hover:border-line-strong",
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
            setAddress(displayAddress(browser.page.url));
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void submit();
            } else if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              setEditing(false);
              setAddress(displayAddress(browser.page.url));
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

      <BrowserSessionMenu
        browser={browser}
        workspaceId={workspaceId}
        autoOpen={autoOpen}
        full={full}
        docked={docked}
        onToggleAutoOpen={onToggleAutoOpen}
        onToggleFull={onToggleFull}
        onToggleDocked={onToggleDocked}
        onResetRatio={onResetRatio}
        onClearData={onClearData}
        onOpenSettings={onOpenSettings}
        onClosePanel={onClose}
        onOpenChange={onMenuOpenChange}
      />
    </div>
  );
}
