import { ACCENTS } from "@/lib/accent";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";

/**
 * A row of colour swatches for the accent ramp.
 *
 * Deliberately not a `<Select>`: the value *is* a colour, so showing the
 * choices as the colours themselves communicates more than a list of names.
 * Each swatch is a real toggle button (`aria-pressed`) inside a labelled
 * group, so the choice is usable and announced without a mouse.
 */
export function AccentPicker() {
  const accent = useUi((state) => state.accent);
  const setAccent = useUi((state) => state.setAccent);
  const theme =
    typeof document !== "undefined" &&
    document.documentElement.dataset.theme === "dark"
      ? "dark"
      : "light";

  return (
    <div
      className="flex flex-wrap items-center gap-2.5"
      role="group"
      aria-label={t("アクセントカラーを選択")}
    >
      {ACCENTS.map((option) => {
        const selected = option.id === accent;
        return (
          <button
            key={option.id}
            type="button"
            aria-label={t(option.label)}
            aria-pressed={selected}
            title={t(option.label)}
            onClick={() => setAccent(option.id)}
            className={cn(
              "size-7 shrink-0 cursor-pointer rounded-full border border-line",
              "transition-[transform,box-shadow] duration-100",
              "hover:scale-105 active:scale-95",
              selected && "ring-2 ring-brand ring-offset-2 ring-offset-surface",
            )}
            style={{ background: option.palette[theme].brand }}
          />
        );
      })}
    </div>
  );
}
