import {
  memo,
  useDeferredValue,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  Brain,
  Check,
  Eye,
  EyeOff,
  Plus,
  Search,
  SlidersHorizontal,
  Sparkles,
  Star,
  Wrench,
  X,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import type { ModelView, ProviderView } from "@/lib/types";
import {
  getFavoriteModels,
  getRecentModels,
  recordRecentModel,
  subscribeModelPrefs,
  toggleFavoriteModel,
} from "@/lib/modelPrefs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ListboxSkeleton } from "@/components/common";
import { EmptyState } from "@/components/shared";
import { SectionLabel } from "@/components/layout";
import { Switch } from "@/components/ui/switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { DropdownMenuTriggerButton } from "@/components/ui/dropdown-menu";

/** Capability icons for a model, with accessible names.
 *
 * A model's three booleans are the reason to pick one over another, but three
 * text badges do not fit beside a long label in a compact row. Icons do, so
 * each carries its name in an `aria-label` and a `title` for the pointer. */
const Capabilities = memo(function Capabilities({
  model,
}: {
  model: ModelView;
}) {
  const caps = [
    model.supports_vision && { key: "vision", Icon: Eye, label: t("Vision") },
    model.supports_tool_calling && {
      key: "tools",
      Icon: Wrench,
      label: t("ツール"),
    },
    model.supports_reasoning && {
      key: "reasoning",
      Icon: Brain,
      label: t("推論"),
    },
  ].filter(Boolean) as { key: string; Icon: typeof Eye; label: string }[];

  if (caps.length === 0) return null;
  return (
    <span className="flex shrink-0 items-center gap-1.5 text-faint">
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
  | {
      kind: "model";
      key: string;
      model: ModelView;
      provider: string;
      /** True in the mixed Recent/Favourites sections, where the provider is
       *  not already implied by a group header. */
      showProvider: boolean;
    };

/** The Docker-edition Auto entry: one pseudo-model that resolves per request. */
export const AUTO_MODEL_ID = "auto";

const LOCAL_PROVIDER_KINDS = new Set(["ollama", "lmstudio"]);

/** A best-effort "this costs nothing" hint.
 *
 * The backend does not model pricing per row, so only two cases are certain:
 * a locally hosted runtime, and a model whose own name says it is free. The
 * badge is advisory — it never changes routing or selection. */
function isFreeModel(model: ModelView, provider: ProviderView | undefined) {
  if (provider && LOCAL_PROVIDER_KINDS.has(provider.kind.toLocaleLowerCase())) {
    return true;
  }
  return /free|無料/i.test(model.id) || /free|無料/i.test(model.label);
}

function providerLabel(model: ModelView, providers: ProviderView[]): string {
  return (
    providers.find((p) => p.id === model.provider_id)?.label ??
    model.provider_id
  );
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
  runtimeStatus,
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
  /** Local runtime state for the selected model. Rendered inside the picker so
   *  the composer toolbar does not need a separate status chip. */
  runtimeStatus?: ReactNode;
}) {
  const openSettings = useUi((s) => s.openSettings);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  // Defer filtering so keystrokes stay responsive with hundreds of models.
  const deferredQuery = useDeferredValue(query);
  const [active, setActive] = useState(0);
  // The list opens on the current selection without painting a highlight on it:
  // a filled row would compete with the check mark and make the panel look like
  // it has two selections. The highlight appears once the keyboard is used.
  const [keyboardNavigating, setKeyboardNavigating] = useState(false);
  const [favorites, setFavorites] = useState<string[]>([]);
  const [recents, setRecents] = useState<string[]>([]);
  const listRef = useRef<HTMLDivElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const listboxId = useId();

  const selected =
    value === AUTO_MODEL_ID
      ? null
      : (models.find((m) => m.id === value) ?? null);
  const isAutoSelected = value === AUTO_MODEL_ID;

  // Preferences live in `localStorage`; refresh them whenever the dialog opens
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

  const providerById = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider])),
    [providers],
  );

  const rows = useMemo<Row[]>(() => {
    const needle = deferredQuery.trim().toLocaleLowerCase();
    const autoMatches =
      !needle ||
      "auto".includes(needle) ||
      t("Auto").toLocaleLowerCase().includes(needle);
    const matches = (m: ModelView) => {
      if (!needle) return true;
      return [m.label, m.id, providerLabel(m, providers)].some((v) =>
        v.toLocaleLowerCase().includes(needle),
      );
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
        out.push({
          kind: "model",
          key: model.id,
          model,
          provider,
          showProvider: false,
        });
      }
      return out;
    }

    const byId = new Map(models.map((m) => [m.id, m]));
    const seen = new Set<string>();
    // Auto always leads: it is the default pick, not just another provider row.
    const out: Row[] = [{ kind: "auto", key: "auto" }];
    const pushModel = (model: ModelView, showProvider: boolean) => {
      if (seen.has(model.id)) return;
      seen.add(model.id);
      out.push({
        kind: "model",
        key: model.id,
        model,
        provider: providerLabel(model, providers),
        showProvider,
      });
    };

    const favoriteModels = favorites
      .map((id) => byId.get(id))
      .filter((m): m is ModelView => m != null);
    if (favoriteModels.length > 0) {
      out.push({ kind: "header", key: "h:favorites", label: t("お気に入り") });
      favoriteModels.forEach((model) => pushModel(model, true));
    }

    const recentModels = recents
      .map((id) => byId.get(id))
      .filter((m): m is ModelView => m != null && !favorites.includes(m.id));
    if (recentModels.length > 0) {
      out.push({
        kind: "header",
        key: "h:recents",
        label: t("最近使ったモデル"),
      });
      recentModels.forEach((model) => pushModel(model, true));
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
      group.forEach((model) => pushModel(model, false));
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
    setKeyboardNavigating(false);
  }, [query, open]);

  useEffect(() => {
    if (active >= optionRows.length)
      setActive(Math.max(0, optionRows.length - 1));
  }, [active, optionRows.length]);

  // Keep the highlighted row inside the scroll viewport. `scrollIntoView` is
  // not implemented in jsdom, so the optional call keeps the test suite quiet.
  useEffect(() => {
    if (!open) return;
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-index="${active}"]`,
    );
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
    // Enter is also how an IME commits a Japanese conversion. Choosing the
    // highlighted model then silently changed the selection and closed the
    // picker while the user was still typing a search term.
    if (
      event.nativeEvent.isComposing ||
      event.keyCode === 229 ||
      (event.key === "Enter" && event.nativeEvent.keyCode === 229)
    ) {
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setKeyboardNavigating(true);
      setActive((index) => Math.min(index + 1, optionRows.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setKeyboardNavigating(true);
      setActive((index) => Math.max(index - 1, 0));
    } else if (event.key === "Home") {
      event.preventDefault();
      setKeyboardNavigating(true);
      setActive(0);
    } else if (event.key === "End") {
      event.preventDefault();
      setKeyboardNavigating(true);
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
    : (selected?.label ??
      (pending
        ? t("読み込み中…")
        : models.length === 0
          ? t("モデル未設定")
          : t("モデル")));

  let optionIndex = -1;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild disabled={disabled}>
        <DropdownMenuTriggerButton
          aria-label={t("モデルを選択")}
          aria-expanded={open}
          title={selected?.label ?? triggerLabel}
          className={cn(
            "border-transparent bg-transparent hover:bg-hover",
            className,
          )}
        >
          <span className="min-w-0 flex-1 truncate text-start">
            {triggerLabel}
          </span>
          {isAutoSelected && (
            <span
              className="flex shrink-0 items-center text-brand"
              title={t("リクエストごとに最適なモデルを選択")}
            >
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
      </DialogTrigger>
      <DialogContent
        aria-label={t("モデルを選択")}
        showClose={false}
        onOpenAutoFocus={(event) => {
          // Radix would focus the content container; the search field is where
          // the user is about to type, and focusing it also keeps the keyboard
          // list navigation live from the first keystroke.
          event.preventDefault();
          searchRef.current?.focus();
        }}
        className={cn(
          "top-[8vh] flex max-h-[84vh] w-[calc(100vw-2rem)] max-w-3xl translate-y-0 flex-col",
          "gap-0 overflow-hidden p-0",
        )}
      >
        <DialogTitle className="sr-only">{t("モデルを選択")}</DialogTitle>
        <DialogDescription className="sr-only">
          {t("モデルを検索")}
        </DialogDescription>

        <div className="flex shrink-0 items-center gap-2 border-b border-line px-4 py-2.5">
          <Search
            size={18}
            className="shrink-0 text-faint"
            aria-hidden="true"
          />
          <input
            ref={searchRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            role="combobox"
            aria-expanded="true"
            aria-controls={listboxId}
            aria-activedescendant={
              optionRows.length > 0
                ? `${listboxId}-option-${active}`
                : undefined
            }
            aria-autocomplete="list"
            aria-label={t("モデルを検索")}
            placeholder={t("モデルを検索")}
            className="min-w-0 flex-1 bg-transparent py-1 text-base text-fg outline-none placeholder:text-faint"
          />
          {query && (
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => setQuery("")}
              aria-label={t("クリア")}
              title={t("クリア")}
            >
              <X size={14} />
            </Button>
          )}
          {/* With no models the empty state already offers "add provider";
              a second button with the same accessible name would make the
              action ambiguous to screen readers and tests alike. */}
          {models.length > 0 && (
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => {
                setOpen(false);
                openSettings("providers");
              }}
              aria-label={t("プロバイダーを追加")}
              title={t("プロバイダーを追加")}
            >
              <Plus size={17} />
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => {
              setOpen(false);
              openSettings("models");
            }}
            aria-label={t("モデルを管理")}
            title={t("モデルを管理")}
          >
            <SlidersHorizontal size={16} />
          </Button>
        </div>

        <div
          id={listboxId}
          ref={listRef}
          role="listbox"
          aria-label={t("モデル")}
          className="flex min-h-0 flex-1 flex-col overflow-y-auto p-1.5"
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
                  <SectionLabel
                    key={row.key}
                    className="px-3 pt-3 pb-1 ps-10 text-xs font-medium tracking-normal text-muted normal-case"
                  >
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
                    id={`${listboxId}-option-${index}`}
                    data-index={index}
                    role="option"
                    aria-selected={isSelected}
                    onMouseEnter={() => {
                      setKeyboardNavigating(false);
                      setActive(index);
                    }}
                    onFocus={() => setActive(index)}
                    onClick={() => choose(AUTO_MODEL_ID)}
                    className={cn(
                      "group flex w-full cursor-pointer items-center gap-2.5 rounded-md px-3 py-2 text-start",
                      "transition-colors duration-75",
                      isActive && keyboardNavigating
                        ? "bg-active"
                        : "hover:bg-hover",
                    )}
                  >
                    <span className="flex size-4 shrink-0 items-center justify-center">
                      {isSelected && (
                        <Check
                          size={15}
                          className="text-fg"
                          aria-hidden="true"
                        />
                      )}
                    </span>
                    <span className="shrink-0 text-sm font-medium text-fg">
                      {t("Auto")}
                    </span>
                    <span className="min-w-0 truncate text-sm text-muted">
                      {t("リクエストごとに最適なモデルを選択")}
                    </span>
                    <Sparkles
                      size={13}
                      className="ms-auto shrink-0 text-brand"
                      aria-hidden="true"
                    />
                  </button>
                );
              }
              optionIndex += 1;
              const index = optionIndex;
              const isActive = index === active;
              const isSelected = row.model.id === value;
              const isFavorite = favorites.includes(row.model.id);
              const provider = providerById.get(row.model.provider_id);
              const free = isFreeModel(row.model, provider);
              return (
                // Wrapper carries the highlight; the option itself is a native
                // button (keyboard handling comes free) and the favourite star
                // is its sibling — never nested interactive content.
                <div
                  key={row.key}
                  data-index={index}
                  onMouseEnter={() => {
                    setKeyboardNavigating(false);
                    setActive(index);
                  }}
                  className={cn(
                    "group flex items-center gap-1 rounded-md px-1",
                    "transition-colors duration-75",
                    isActive && keyboardNavigating
                      ? "bg-active"
                      : "hover:bg-hover",
                  )}
                >
                  <button
                    type="button"
                    id={`${listboxId}-option-${index}`}
                    role="option"
                    aria-selected={isSelected}
                    onFocus={() => setActive(index)}
                    onClick={() => choose(row.model.id)}
                    title={`${row.model.label} · ${row.provider}`}
                    className="flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-start outline-none"
                  >
                    <span className="flex size-4 shrink-0 items-center justify-center">
                      {isSelected && (
                        <Check
                          size={15}
                          className="text-fg"
                          aria-hidden="true"
                        />
                      )}
                    </span>
                    <span className="flex min-w-0 flex-1 items-center gap-2">
                      <span className="truncate text-sm font-medium text-fg">
                        {row.model.label}
                      </span>
                      {row.showProvider && (
                        <span className="shrink-0 text-sm text-muted">
                          {row.provider}
                        </span>
                      )}
                      {free && (
                        <Badge size="sm" className="shrink-0 rounded-sm">
                          {t("無料")}
                        </Badge>
                      )}
                    </span>
                  </button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => onToggleFavorite(row.model.id)}
                    // Keep focus in the search box so arrow-key travel is not
                    // interrupted when toggling a favourite with the pointer.
                    onMouseDown={(e) => e.preventDefault()}
                    aria-label={
                      isFavorite
                        ? t("お気に入りから外す")
                        : t("お気に入りに追加")
                    }
                    title={
                      isFavorite
                        ? t("お気に入りから外す")
                        : t("お気に入りに追加")
                    }
                    selected={isFavorite}
                    className={cn(
                      "-me-0.5 shrink-0 text-faint opacity-0 transition-opacity",
                      "group-hover:opacity-100 focus-visible:opacity-100",
                      isFavorite && "opacity-100",
                    )}
                  >
                    <Star
                      size={13}
                      fill={isFavorite ? "currentColor" : "none"}
                      aria-hidden="true"
                    />
                  </Button>
                </div>
              );
            })
          )}
        </div>

        {(onTemporaryChange || runtimeStatus) && (
          <div className="shrink-0 border-t border-line px-4 py-3">
            {onTemporaryChange && (
              <>
                <label className="flex items-center justify-between gap-2 text-xs text-fg">
                  <span className="inline-flex items-center gap-1.5">
                    <EyeOff
                      size={12}
                      className="text-muted"
                      aria-hidden="true"
                    />
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
                      {t(
                        "この送信ではMemoryを使いません。Run終了後に会話・回答・送信した添付を削除します。",
                      )}
                    </p>
                    <label className="mt-2 flex items-center justify-between gap-2 text-xs text-fg">
                      <span className="inline-flex items-center gap-1.5">
                        <Wrench
                          size={12}
                          className="text-muted"
                          aria-hidden="true"
                        />
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
              </>
            )}
            {runtimeStatus && (
              <div className="mt-2 flex items-center gap-2">
                {runtimeStatus}
              </div>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
