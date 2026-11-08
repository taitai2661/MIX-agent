import { useState } from "react";
import {
  ArrowLeft,
  Folder,
  MessageSquare,
  Plug,
  ShieldCheck,
  Sparkles,
  X,
  type LucideIcon,
} from "lucide-react";
import { t, tf } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import { StepFolder } from "./StepFolder";
import { StepProvider } from "./StepProvider";
import { StepRail, type OnboardingStepMeta } from "./StepRail";
import { StepStart } from "./StepStart";
import { StepWelcome } from "./StepWelcome";
import { WindowControls } from "../shell/WindowControls";

interface StepConfig extends OnboardingStepMeta {
  heading: string;
  description: string;
  asideTitle: string;
  asideBody: string;
  Icon: LucideIcon;
}

/** Steps the rail shows. The welcome page is not one of them: it has no state
 * behind it, so numbering it would promise progress that cannot happen. */
const STEPS: StepConfig[] = [
  {
    id: "provider",
    title: "モデル提供元",
    heading: "モデルを接続",
    description:
      "このアプリの頭脳になるモデルを選びます。ローカルでもクラウドでもかまいません。",
    asideTitle: "頭脳をつなぐ",
    asideBody:
      "LM Studio や Ollama、またはクラウド API。あなたの環境に合うモデルを選べます。",
    Icon: Plug,
  },
  {
    id: "folder",
    title: "作業フォルダ",
    heading: "作業フォルダを選ぶ",
    description:
      "エージェントがファイルを読み書きする場所を決めます。あとから変更できます。",
    asideTitle: "作業場所を決める",
    asideBody: "指定したフォルダの中だけが、エージェントの作業範囲になります。",
    Icon: Folder,
  },
  {
    id: "start",
    title: "チャット",
    heading: "準備ができました",
    description: "設定を確認して、最初のスレッドを開きましょう。",
    asideTitle: "さあ、始めましょう",
    asideBody:
      "設定はいつでも変更できます。まずは気軽に話しかけてみてください。",
    Icon: MessageSquare,
  },
];

const WELCOME_ASIDE = {
  asideTitle: "ローカルで動く AI ワークスペース",
  asideBody:
    "会話もファイルも、あなたの端末の中に。プライバシーを保ったまま、日常の作業を任せられます。",
  Icon: Sparkles,
};

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
 *
 * The window is split into a contextual rail and a task surface. The rail keeps
 * the brand, the current step's intent and the progress visible without turning
 * setup into a form full of numbered fields.
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

  const aside = step === WELCOME ? WELCOME_ASIDE : STEPS[step];
  const AsideIcon = aside.Icon;
  const eyebrow = step === WELCOME ? "LOCAL-FIRST" : `STEP 0${step + 1}`;

  return (
    <div className="flex h-full min-w-0 flex-col bg-bg">
      <header
        data-tauri-drag-region
        className="app-titlebar relative z-20 flex h-titlebar shrink-0 items-center gap-2 border-b border-line bg-bg/85 px-3 backdrop-blur-md"
      >
        <span
          data-tauri-drag-region
          className="flex min-w-0 items-center gap-2"
        >
          <span className="grid size-5 shrink-0 place-items-center rounded-md bg-brand text-on-solid">
            <Sparkles size={11} aria-hidden="true" />
          </span>
          <span className="truncate text-sm font-medium text-fg">
            MIX-agent
          </span>
          <span className="hidden text-2xs text-faint sm:inline">
            / {t("初回セットアップ")}
          </span>
        </span>
        <span className="flex-1" />
        {step > WELCOME && (
          <span className="text-2xs tabular-nums text-faint lg:hidden">
            {tf("ステップ {n} / {total}", {
              n: step + 1,
              total: STEPS.length,
            })}
          </span>
        )}
        <Button
          variant="ghost"
          size="sm"
          onClick={onDefer}
          title={t("あとで設定する")}
          aria-label={t("あとで設定する")}
        >
          <X size={12} />
          <span className="hidden sm:inline">{t("あとで設定する")}</span>
        </Button>
        <WindowControls />
      </header>

      <div className="flex min-h-0 min-w-0 flex-1">
        {/* The rail is deliberately not visible on narrow windows: the step
            surface already says where you are, and the progress belongs beside
            the task rather than above it. */}
        <aside className="relative hidden w-[348px] shrink-0 flex-col overflow-hidden border-e border-line bg-sidebar lg:flex xl:w-[392px]">
          <div
            aria-hidden="true"
            className="pointer-events-none absolute -top-28 -left-24 size-80 rounded-full bg-brand/15 blur-3xl"
          />
          <div
            aria-hidden="true"
            className="pointer-events-none absolute -right-28 bottom-10 size-72 rounded-full bg-brand-strong/10 blur-3xl"
          />

          <div className="relative flex min-h-0 flex-1 flex-col overflow-y-auto px-8 py-8">
            <div className="flex items-center gap-3">
              <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-gradient-to-br from-brand to-brand-strong text-on-solid shadow-sm">
                <Sparkles size={16} aria-hidden="true" />
              </span>
              <div className="flex min-w-0 flex-col">
                <span className="truncate text-sm font-semibold tracking-tight text-fg">
                  MIX-agent
                </span>
                <span className="text-2xs text-faint">{t("デスクトップ")}</span>
              </div>
            </div>

            <div className="mt-14 flex flex-col gap-4">
              <span className="inline-flex w-fit items-center gap-1.5 rounded-full border border-line bg-surface/70 px-2.5 py-1 text-2xs font-semibold tracking-[0.12em] text-muted shadow-xs">
                <AsideIcon
                  size={12}
                  className="text-brand"
                  aria-hidden="true"
                />
                {eyebrow}
              </span>
              <h2 className="text-2xl font-semibold leading-tight tracking-tight text-fg [word-break:keep-all]">
                {t(aside.asideTitle)}
              </h2>
              <p className="text-sm leading-6 text-muted">
                {t(aside.asideBody)}
              </p>
            </div>

            {step > WELCOME && (
              <div className="mt-10">
                <StepRail
                  steps={STEPS}
                  current={step}
                  furthest={furthest}
                  onSelect={onRailSelect}
                />
              </div>
            )}

            <div className="mt-auto pt-10">
              <div className="flex items-start gap-2.5 rounded-xl border border-line bg-surface/60 p-3.5">
                <ShieldCheck
                  size={15}
                  className="mt-0.5 shrink-0 text-brand"
                  aria-hidden="true"
                />
                <div className="flex min-w-0 flex-col gap-0.5">
                  <span className="text-xs font-medium text-fg">
                    {t("ローカルに保存")}
                  </span>
                  <span className="text-2xs leading-5 text-muted">
                    {t("会話・ファイル・設定はこの端末の中に保存されます。")}
                  </span>
                </div>
              </div>
            </div>
          </div>
        </aside>

        <main className="relative flex min-h-0 min-w-0 flex-1 flex-col">
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-0 h-64 bg-gradient-to-b from-brand-soft/50 to-transparent"
          />
          <div className="relative min-h-0 flex-1 overflow-y-auto stable-scroll">
            <div className="mx-auto flex w-full max-w-2xl flex-col px-6 py-8 sm:px-8 lg:px-12 lg:py-12">
              {step === WELCOME && (
                <StepWelcome onDone={() => setStep(0)} onDefer={onDefer} />
              )}

              {step > WELCOME && (
                <div key={step} className="animate-rise flex flex-col gap-6">
                  <StepIntro step={step} />

                  <div className="rounded-2xl border border-line bg-surface/95 p-5 shadow-sm sm:p-6">
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
                      <StepStart
                        projectName={projectName}
                        onStart={onComplete}
                      />
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>

          {step > WELCOME && (
            <footer className="relative shrink-0 border-t border-line bg-bg/85 px-6 py-3 backdrop-blur-md">
              <div className="mx-auto flex w-full max-w-2xl items-center gap-3">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setStep(step === 0 ? WELCOME : step - 1)}
                >
                  <ArrowLeft size={12} /> {t("戻る")}
                </Button>
                <span className="flex-1" />
                {step === 0 && (
                  <Button variant="ghost" size="sm" onClick={() => setStep(1)}>
                    {t("今はスキップ")}
                  </Button>
                )}
                <span className="hidden text-2xs text-faint sm:inline">
                  {t("設定はあとから変更できます")}
                </span>
              </div>
            </footer>
          )}
        </main>
      </div>
    </div>
  );
}

/** Shared heading for the three task screens. Keeping the eyebrow, title and
 *  description here means the steps themselves stay focused on their fields. */
function StepIntro({ step }: { step: number }) {
  const meta = STEPS[step];
  const Icon = meta.Icon;
  return (
    <header className="flex items-start gap-4">
      <span className="grid size-11 shrink-0 place-items-center rounded-2xl border border-brand/15 bg-brand-soft text-brand">
        <Icon size={19} aria-hidden="true" />
      </span>
      <div className="min-w-0">
        <span className="text-2xs font-semibold uppercase tracking-[0.16em] text-brand">
          {tf("ステップ {n} / {total}", {
            n: step + 1,
            total: STEPS.length,
          })}
        </span>
        <h2 className="mt-1 text-2xl font-semibold tracking-tight text-fg [word-break:keep-all]">
          {t(meta.heading)}
        </h2>
        <p className="mt-1.5 text-sm leading-6 text-muted">
          {t(meta.description)}
        </p>
      </div>
    </header>
  );
}
