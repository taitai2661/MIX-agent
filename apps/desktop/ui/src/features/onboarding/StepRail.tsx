import { Check } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

export interface OnboardingStepMeta {
  id: string;
  title: string;
}

/**
 * Progress indicator for the first-run wizard.
 *
 * A vertical list of real destinations, not a `tablist`: it switches which step
 * is *current* rather than revealing a panel, and `aria-current="step"` is what
 * conveys that.
 *
 * Steps after the current one are disabled because they depend on the ones
 * before them — you cannot pick a model before a provider exists. The rail sits
 * in the contextual sidebar, where it doubles as a map of what setup will ask
 * for.
 */
export function StepRail({
  steps,
  current,
  onSelect,
  furthest,
}: {
  steps: OnboardingStepMeta[];
  current: number;
  furthest: number;
  onSelect: (index: number) => void;
}) {
  return (
    <nav aria-label={t("セットアップの進行状況")}>
      <ol className="flex flex-col gap-1">
        {steps.map((step, index) => {
          const done = index < current;
          const reachable = index <= furthest;
          const selected = index === current;
          return (
            <li key={step.id} className="relative">
              {index < steps.length - 1 && (
                <span
                  aria-hidden="true"
                  className={cn(
                    "absolute left-6 top-[39px] -bottom-3 w-px",
                    done ? "bg-brand/35" : "bg-line",
                  )}
                />
              )}
              <button
                type="button"
                aria-current={selected ? "step" : undefined}
                disabled={!reachable}
                onClick={() => onSelect(index)}
                data-testid={`onboarding-step-${step.id}`}
                className={cn(
                  "relative flex w-full items-center gap-3 rounded-xl px-2 py-2 text-start transition-colors",
                  reachable && !selected && "hover:bg-hover",
                  selected && "bg-surface shadow-xs ring-1 ring-brand/15",
                  !reachable && "cursor-not-allowed opacity-55",
                )}
              >
                <span
                  aria-hidden="true"
                  className={cn(
                    "grid size-[31px] shrink-0 place-items-center rounded-full border text-2xs font-semibold",
                    done
                      ? "border-brand bg-brand text-on-solid"
                      : selected
                        ? "border-brand bg-brand-soft text-brand"
                        : "border-line-strong bg-inset text-faint",
                  )}
                >
                  {done ? <Check size={12} /> : index + 1}
                </span>
                <span
                  className={cn(
                    "min-w-0 flex-1 truncate text-sm",
                    selected
                      ? "font-medium text-fg"
                      : reachable
                        ? "text-muted"
                        : "text-faint",
                  )}
                >
                  {t(step.title)}
                </span>
                {selected && (
                  <span className="shrink-0 text-2xs text-brand">
                    {t("現在")}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
