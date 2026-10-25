import { useState } from "react";
import { ArrowLeft, X } from "lucide-react";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import { StepFolder } from "./StepFolder";
import { StepProvider } from "./StepProvider";
import { StepRail, type OnboardingStepMeta } from "./StepRail";
import { StepStart } from "./StepStart";
import { StepWelcome } from "./StepWelcome";
import { WindowControls } from "../shell/WindowControls";

/** Steps the rail shows. The welcome page is not one of them: it has no state
 * behind it, so numbering it would promise progress that cannot happen. */
const STEPS: OnboardingStepMeta[] = [
  { id: "provider", title: "モデル提供元" },
  { id: "folder", title: "作業フォルダ" },
  { id: "start", title: "チャット" },
];

/** Index into the rail. Welcome is `-1`, which reads as "before step 1" for
 * both the rail's `index < current` test and the Back button. */
const WELCOME = -1;

/**
 * The first-run wizard, shown in place of the whole shell.
 *
 * A takeover rather than a modal over the app: a full-page target means the
 * shell's `⌘K`, the composer's disabled send button and the empty sidebar are
 * never rendered in a state where the user can reach them and get nothing, and
 * three questions do not need a scrim between them and the desktop behind.
 */
export function Onboarding({
  onComplete,
  onDefer,
}: {
  onComplete: () => void;
  onDefer: () => void;
}) {
  const [step, setStep] = useState(WELCOME);
  // The folder is chosen in step 2 but summarised in step 3, and step 3 has no
  // back button that would re-run the pick, so the result is held here.
  const [projectName, setProjectName] = useState<string | null>(null);

  // Reaching a step is a consequence of the previous one having succeeded, so
  // the rail only lets the user go back, never skip ahead.
  const furthest = Math.max(step, 0);
  const onRailSelect = (index: number) => {
    if (index <= furthest) setStep(index);
  };

  return (
    <div className="flex h-full min-w-0 flex-col bg-bg">
      <header className="app-titlebar flex h-titlebar shrink-0 items-center gap-1.5 border-b border-line bg-bg/85 px-3 backdrop-blur-md">
        <span className="truncate text-sm font-medium text-fg">MIX-agent</span>
        {step > WELCOME && (
          <StepRail
            steps={STEPS}
            current={step}
            furthest={furthest}
            onSelect={onRailSelect}
          />
        )}
        <span className="flex-1" />
        <Button
          variant="ghost"
          size="sm"
          onClick={onDefer}
          title={t("あとで設定する")}
          aria-label={t("あとで設定する")}
        >
          <X size={12} />
          {t("あとで設定する")}
        </Button>
        <WindowControls />
      </header>

      <main className="relative flex min-h-0 flex-1 justify-center overflow-y-auto px-6 py-10">
        <div
          aria-hidden="true"
          className="pointer-events-none absolute left-1/2 top-0 size-[36rem] -translate-x-1/2 rounded-full bg-brand/[0.07] blur-3xl"
        />
        <div className="relative w-full max-w-xl">
          {step === WELCOME && <StepWelcome onDone={() => setStep(0)} />}

          {step === 0 && <StepProvider onDone={() => setStep(1)} />}

          {step === 1 && (
            <StepFolder
              onDone={(name) => {
                setProjectName(name);
                setStep(2);
              }}
              onSkip={() => {
                setProjectName(null);
                setStep(2);
              }}
            />
          )}

          {step === 2 && (
            <StepStart projectName={projectName} onStart={onComplete} />
          )}
        </div>
      </main>

      {step > WELCOME && (
        <footer className="flex shrink-0 items-center gap-2 border-t border-line px-6 py-3">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setStep(step === 0 ? WELCOME : step - 1)}
          >
            <ArrowLeft size={12} /> {t("戻る")}
          </Button>
          <span className="flex-1" />
          <span className="text-xs text-faint">
            {t("設定はあとから変更できます")}
          </span>
        </footer>
      )}
    </div>
  );
}
