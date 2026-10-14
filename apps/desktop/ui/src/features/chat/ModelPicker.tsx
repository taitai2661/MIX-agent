import { memo, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import {
  Brain,
  Check,
  Eye,
  EyeOff,
  Search,
  Settings2,
  Sparkles,
  Star,
  Wrench,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { t, tf } from "@/lib/i18n";
import { formatContext, shortModelId } from "@/lib/format";
import { useUi } from "@/lib/ui";
import type { ModelView, ProviderView } from "@/lib/types";
import {
  getFavoriteModels,
  getRecentModels,
  recordRecentModel,
  subscribeModelPrefs,
  toggleFavoriteModel,
} from "@/lib/modelPrefs";
import { Button } from "@/components/ui/button";
import { ListboxSkeleton } from "@/components/common";
import { EmptyState } from "@/components/shared";
import { SectionLabel } from "@/components/layout";
import { Switch } from "@/components/ui/switch";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { SearchField } from "@/components/ui/input";
import { DropdownMenuTriggerButton } from "@/components/ui/dropdown-menu";

/** Capability icons for a model, with accessible names.
 *
 * A model's three booleans are the reason to pick one over another, but three
 * text badges do not fit beside a long label in a 20rem popover. Icons do, so
 * each carries its name in an `aria-label` and a `title` for the pointer. */
const Capabilities = memo(function Capabilities({ model }: { model: ModelView }) {
  const caps = [
    model.supports_vision && { key: "vision", Icon: Eye, label: t("Vision") },
    model.supports_tool_calling && { key: "tools", Icon: Wrench, label: t("ツール") },
    model.supports_reasoning && { key: "reasoning", Icon: Brain, label: t("推論") },
  ].filter(Boolean) as { key: string; Icon: typeof Eye; label: string }[];

  if (caps.length === 0) return null;
  // Always rendered: the trigger width is capped with truncation, so the
  // icons never appear/disappear with the column width (no container query).
  return (
    <span className="flex shrink-0 items-center gap-1.5 text-muted">
      {caps.map(({ key, Icon, label }) => (
        <span key={key} title={label} aria-label={label} role="img">
          <Icon size={13} aria-hidden="true" />
        </span>
      ))}
    </span>
  );
});

type Row =
  | { kind: "header"; key: string; label: string }
  | { kind: "auto"; key: string }
  | { kind: "model"; key: string; model: ModelView; provider: string };

/** The Docker-edition Auto entry: one pseudo-model that resolves per request. */
export const AUTO_MODEL_ID = "auto";

function AutoRowLabel() {
  return (
    <span className="min-w-0 flex-1">
      <span className="flex items-center gap-1.5">
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-fg">
          {t("Auto")}
        </span>
        <span className="flex shrink-0 items-center gap-1 text-faint">
          <Sparkles size={12} aria-hidden="true" />
        </span>
      </span>
      <span className="mt-0.5 flex items-center gap-2 text-2xs text-faint">
        <span className="min-w-0 truncate">
          {t("リクエストごとに最適なモデルを選択")}
        </span>
      </span>
    </span>
  );
}

const AutoRowLabelMemo = memo(AutoRowLabel);

function providerLabel(model: ModelView, providers: ProviderView[]): string {
  return providers.find((p) => p.id === model.provider_id)?.label ?? model.provider_id;
}

export function ModelPicker({
  models,
  value,
  onChange,
  providers,
  pending = false,
  disabled = false,
  className,
  temporary,
  onTemporaryChange,
  allowTools,
  onAllowToolsChange,
}: {
  models: ModelView[];
  value: string;
  onChange: (id: string) => void;
  providers: ProviderView[];
  pending?: boolean;
  disabled?: boolean;
  className?: string;
  /** Docker-edition temporary mode. The toggles only render when the caller
   *  supplies a handler, so the settings pane can reuse the picker without
   *  exposing per-send options that do not apply there. */
  temporary?: boolean;
  onTemporaryChange?: (value: boolean) => void;
  allowTools?: boolean;
  onAllowToolsChange?: (value: boolean) => void;
}) {
  const openSettings = useUi((s) => s.openSettings);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  // Defer filtering so keystrokes stay responsive with hundreds of models.
  const deferredQuery = useDeferredValue(query);
  const [active, setActive] = useState(0);
  const [favorites, setFavorites] = useState<string[]>([]);
  const [recents, setRecents] = useState<string[]>([]);
  const listRef = useRef<HTMLDivElement | null>(null);

  const selected = value === AUTO_MODEL_ID
    ? null
    : (models.find((m) => m.id === value) ?? null);
  const isAutoSelected = value === AUTO_MODEL_ID;

  // Preferences live in `localStorage`; refresh them whenever the popover opens
  // or another picker toggles a favourite while this one is mounted.
  useEffect(() => {
    const refresh = () => {
      setFavorites(getFavoriteModels());
      setRecents(getRecentModels());
    };
    refresh();
    return subscribeModelPrefs(refresh);
  }, []);

  useEffect(() => {
    if (open) {
      setFavorites(getFavoriteModels());
      setRecents(getRecentModels());
    } else {
      setQuery("");
    }
  }, [open]);

  const rows = useMemo<Row[]>(() => {
    const needle = deferredQuery.trim().toLocaleLowerCase();
    const autoMatches =
      !needle ||
      "auto".includes(needle) ||
      t("Auto").toLocaleLowerCase().includes(needle);
    const matches = (m: ModelView) => {
      if (!needle) return true;
      return [m.label, m.id, providerLabel(m, providers)]
        .some((v) => v.toLocaleLowerCase().includes(needle));
    };

    if (needle) {
      const out: Row[] = [];
      if (autoMatches) out.push({ kind: "auto", key: "auto" });
      let lastProvider: string | null = null;
      for (const model of models) {
        if (!matches(model)) continue;
        const provider = providerLabel(model, providers);
        if (provider !== lastProvider) {
          out.push({ kind: "header", key: `h:${provider}`, label: provider });
          lastProvider = provider;
        }
        out.push({ kind: "model", key: model.id, model, provider });
      }
      return out;
    }

    const byId = new Map(models.map((m) => [m.id, m]));
    const seen = new Set<string>();
    // Auto always leads: it is the default pick, not just another provider row.
    const out: Row[] = [{ kind: "auto", key: "auto" }];
    const pushModel = (model: ModelView) => {
      if (seen.has(model.id)) return;
      seen.add(model.id);
      out.push({ kind: "model", key: model.id, model, provider: providerLabel(model, providers) });
    };

    const favoriteModels = favorites
      .map((id) => byId.get(id))
      .filter((m): m is ModelView => m != null);
    if (favoriteModels.length > 0) {
      out.push({ kind: "header", key: "h:favorites", label: t("お気に入り") });
      favoriteModels.forEach(pushModel);
    }

    const recentModels = recents
      .map((id) => byId.get(id))
      .filter((m): m is ModelView => m != null && !favorites.includes(m.id));
    if (recentModels.length > 0) {
      out.push({ kind: "header", key: "h:recents", label: t("最近使ったモデル") });
      recentModels.forEach(pushModel);
    }

    // Everything left, grouped by provider in list order.
    const groups = new Map<string, ModelView[]>();
    for (const model of models) {
      if (seen.has(model.id)) continue;
      const provider = providerLabel(model, providers);
      const bucket = groups.get(provider);
      if (bucket) bucket.push(model);
      else groups.set(provider, [model]);
    }
    for (const [provider, group] of groups) {
      out.push({ kind: "header", key: `h:${provider}`, label: provider });
      group.forEach(pushModel);
    }
    return out;
  }, [deferredQuery, favorites, models, providers, recents]);

  // The keyboard walks a flat list of options; headers are presentational.
  const optionRows = useMemo(
    () =>
      rows.filter(
        (row): row is Extract<Row, { kind: "model" } | { kind: "auto" }> =>
          row.kind === "model" || row.kind === "auto",
      ),
    [rows],
  );

  useEffect(() => {
    setActive(0);
  }, [query, open]);

  useEffect(() => {
    if (active >= optionRows.length) setActive(Math.max(0, optionRows.length - 1));
  }, [active, optionRows.length]);

  // Keep the highlighted row inside the scroll viewport. `scrollIntoView` is
  // not implemented in jsdom, so the optional call keeps the test suite quiet.
  useEffect(() => {
    if (!open) return;
    const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    el?.scrollIntoView?.({ block: "nearest" });
  }, [active, open]);

  const choose = (id: string) => {
    onChange(id);
    setRecents(recordRecentModel(id));
    setOpen(false);
  };

  const onToggleFavorite = (id: string) => {
    setFavorites(toggleFavoriteModel(id));
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((index) => Math.min(index + 1, optionRows.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((index) => Math.max(index - 1, 0));
    } else if (event.key === "Home") {
      event.preventDefault();
      setActive(0);
    } else if (event.key === "End") {
      event.preventDefault();
      setActive(Math.max(0, optionRows.length - 1));
    } else if (event.key === "Enter") {
      const row = optionRows[active];
      if (row) {
        event.preventDefault();
        choose(row.kind === "auto" ? AUTO_MODEL_ID : row.model.id);
      }
    }
  };

  const triggerLabel = isAutoSelected
    ? t("Auto")
    : (selected?.label
      ?? (pending ? t("読み込み中…") : models.length === 0 ? t("モデル未設定") : t("モデル")));

  let optionIndex = -1;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild disabled={disabled}>
        <DropdownMenuTriggerButton
          aria-label={t("モデルを選択")}
          aria-expanded={open}
          title={selected?.label ?? triggerLabel}
          className={cn("border-transparent bg-transparent hover:bg-hover", className)}
        >
          <span className="min-w-0 flex-1 truncate text-start">{triggerLabel}</span>
          {isAutoSelected && (
            <span className="flex shrink-0 items-center text-brand" title={t("リクエストごとに最適なモデルを選択")}>
              <Sparkles size={12} aria-hidden="true" />
            </span>
          )}
          {temporary && (
            <span
              role="img"
              className="shrink-0 text-warning"
              title={t("一時モード")}
              aria-label={t("一時モード")}
            >
              <EyeOff size={12} aria-hidden="true" />
            </span>
          )}
          {selected && <Capabilities model={selected} />}
        </DropdownMenuTriggerButton>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        aria-label={t("モデルを選択")}
        className="flex w-[22rem] max-w-[calc(100vw-1rem)] flex-col"
      >
        <div className="shrink-0 border-b border-line p-1.5">
          <SearchField
            // APG menu pattern: focus enters the popover on open so keyboard
            // users land in the search, not behind the overlay.
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
            icon={<Search size={13} />}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            onClear={query ? () => setQuery("") : undefined}
            clearLabel={t("クリア")}
            role="combobox"
            aria-expanded="true"
            aria-controls="model-picker-listbox"
            aria-activedescendant={
              optionRows.length > 0 ? `model-option-${active}` : undefined
            }
            aria-autocomplete="list"
            aria-label={t("モデルを検索")}
            placeholder={t("モデルを検索")}
          />
        </div>

        <div
          id="model-picker-listbox"
          ref={listRef}
          role="listbox"
          aria-label={t("モデル")}
          className="flex max-h-80 min-h-0 flex-col overflow-y-auto p-1.5"
        >
          {pending && models.length === 0 ? (
            <ListboxSkeleton />
          ) : models.length === 0 ? (
            <EmptyState
              compact
              title={t("モデルがありません")}
              sub={t("プロバイダーを追加してモデルを取得してください")}
              action={
                <Button
                  size="sm"
                  variant="primary"
                  onClick={() => {
                    setOpen(false);
                    openSettings("providers");
                  }}
                >
                  {t("プロバイダーを追加")}
                </Button>
              }
            />
          ) : optionRows.length === 0 ? (
            <EmptyState compact title={t("該当するモデルがありません")} />
          ) : (
            rows.map((row) => {
              if (row.kind === "header") {
                return (
                  <SectionLabel key={row.key} className="px-2 pt-2 pb-1">
                    {row.label}
                  </SectionLabel>
                );
              }
              if (row.kind === "auto") {
                optionIndex += 1;
                const index = optionIndex;
                const isActive = index === active;
                const isSelected = value === AUTO_MODEL_ID;
                return (
                  <button
                    key={row.key}
                    type="button"
                    id={`model-option-${index}`}
                    data-index={index}
                    role="option"
                    aria-selected={isSelected}
                    onMouseEnter={() => setActive(index)}
                    onFocus={() => setActive(index)}
                    onClick={() => choose(AUTO_MODEL_ID)}
                    className={cn(
                      "group flex w-full cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 text-start",
                      "transition-colors duration-75",
                      isActive ? "bg-active" : "hover:bg-hover",
                    )}
                  >
                    <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center">
                      {isSelected && (
                        <Check size={13} className="text-brand" aria-hidden="true" />
                      )}
                    </span>
                    <AutoRowLabelMemo />
                  </button>
                );
              }
              optionIndex += 1;
              const index = optionIndex;
              const isActive = index === active;
              const isSelected = row.model.id === value;
              const isFavorite = favorites.includes(row.model.id);
              return (
                // Wrapper carries the highlight; the option itself is a native
                // button (keyboard handling comes free) and the favourite star
                // is its sibling — never nested interactive content.
                <div
                  key={row.key}
                  data-index={index}
                  onMouseEnter={() => setActive(index)}
                  className={cn(
                    "group flex items-start gap-1 rounded-md px-1 py-0.5",
                    "transition-colors duration-75",
                    isActive ? "bg-active" : "hover:bg-hover",
                  )}
                >
                  <button
                    type="button"
                    id={`model-option-${index}`}
                    role="option"
                    aria-selected={isSelected}
                    onFocus={() => setActive(index)}
                    onClick={() => choose(row.model.id)}
                    className="flex min-w-0 flex-1 cursor-pointer items-start gap-2 rounded-md px-1 py-1 text-start outline-none"
                  >
                    <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center">
                      {isSelected && (
                        <Check size={13} className="text-brand" aria-hidden="true" />
                      )}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-1.5">
                        <span className="min-w-0 flex-1 truncate text-sm font-medium text-fg">
                          {row.model.label}
                        </span>
                        <Capabilities model={row.model} />
                      </span>
                      <span className="mt-0.5 flex items-center gap-2 text-2xs text-faint">
                        <span className="min-w-0 truncate font-mono" title={row.model.id}>{shortModelId(row.model.id)}</span>
                        <span className="ms-auto shrink-0 font-mono">
                          {formatContext(row.model.context_window)}
                        </span>
                      </span>
                    </span>
                  </button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => onToggleFavorite(row.model.id)}
                    // Keep focus in the search box so arrow-key travel is not
                    // interrupted when toggling a favourite with the pointer.
                    onMouseDown={(e) => e.preventDefault()}
                    aria-label={isFavorite ? t("お気に入りから外す") : t("お気に入りに追加")}
                    title={isFavorite ? t("お気に入りから外す") : t("お気に入りに追加")}
                    selected={isFavorite}
                    className="-me-0.5 mt-0.5 text-faint"
                  >
                    <Star size={13} fill={isFavorite ? "currentColor" : "none"} aria-hidden="true" />
                  </Button>
                </div>
              );
            })
          )}
        </div>

        {onTemporaryChange && (
          <div className="shrink-0 border-t border-line px-2.5 py-2">
            <label className="flex items-center justify-between gap-2 text-xs text-fg">
              <span className="inline-flex items-center gap-1.5">
                <EyeOff size={12} className="text-muted" aria-hidden="true" />
                {t("一時モード")}
              </span>
              <Switch
                checked={!!temporary}
                onCheckedChange={(checked) => {
                  onTemporaryChange(checked);
                  if (!checked) onAllowToolsChange?.(false);
                }}
                aria-label={t("一時モード")}
              />
            </label>
            {temporary && (
              <>
                <p className="mt-1 text-2xs text-faint">
                  {t("この送信ではMemoryを使いません。Run終了後に会話・回答・送信した添付を削除します。")}
                </p>
                <label className="mt-2 flex items-center justify-between gap-2 text-xs text-fg">
                  <span className="inline-flex items-center gap-1.5">
                    <Wrench size={12} className="text-muted" aria-hidden="true" />
                    {t("Toolを許可")}
                  </span>
                  <Switch
                    checked={!!allowTools}
                    onCheckedChange={onAllowToolsChange}
                    aria-label={t("Toolを許可")}
                  />
                </label>
                {allowTools && (
                  <p className="mt-1 text-2xs text-faint">
                    {t("Toolの外部送信や副作用は残る場合があります。")}
                  </p>
                )}
              </>
            )}
          </div>
        )}

        <div className="flex shrink-0 items-center justify-between gap-2 border-t border-line px-2 py-1.5">
          <span className="text-2xs text-faint">
            {tf("{n} モデル", { n: models.length })}
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setOpen(false);
              openSettings("models");
            }}
          >
            <Settings2 size={12} /> {t("モデルを管理")}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
