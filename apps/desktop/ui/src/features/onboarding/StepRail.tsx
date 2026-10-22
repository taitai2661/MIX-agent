import { Check } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";

export interface OnboardingStepMeta {
  id: string;
  title: string;
}

/**
 * Progress indicator for the first-run wizard.
 *
 * A list of real destinations, not a `tablist`: the rail switches which step is
 * *current* rather than revealing a panel, and `aria-current="step"` is what
 * conveys that. (The sidebar's thread filter had the same mismatch and became a
 * segmented control instead.)
 *
 * Steps after the current one are disabled because they depend on the ones
 * before them — you cannot pick a model before a provider exists.
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
    <ol
      className="flex items-center gap-1.5"
      aria-label={t("セットアップの進行状況")}
    >
      {steps.map((step, index) => {
        const done = index < current;
        const reachable = index <= furthest;
        const selected = index === current;
        return (
          <li key={step.id}>
            <Button
              variant="ghost"
              size="sm"
              aria-current={selected ? "step" : undefined}
              disabled={!reachable}
              onClick={() => onSelect(index)}
              data-testid={`onboarding-step-${step.id}`}
              className={cn(
                "gap-1.5 px-2",
                !reachable && "text-faint",
                reachable && !selected && "text-muted",
              )}
            >
              <span
                aria-hidden="true"
                className={cn(
                  "flex size-5 items-center justify-center rounded-full border text-2xs",
                  done
                    ? "border-brand bg-brand text-on-solid"
                    : selected
                      ? "border-brand text-brand"
                      : "border-line-strong text-faint",
                )}
              >
                {done ? <Check size={9} /> : index + 1}
              </span>
              {t(step.title)}
            </Button>
          </li>
        );
      })}
    </ol>
  );
}
