import { useCallback } from "react";
import { useApplySettings, useSettingsForm } from "@/lib/settings";
import { useProviders } from "@/lib/queries";
import { useUi } from "@/lib/ui";

/**
 * Decides whether the first-run wizard replaces the shell.
 *
 * The question is "has this installation been set up at all?", and it is asked
 * of two things:
 *
 *  - the persisted `onboarding_completed` flag, which is the real answer. It is
 *    set when the wizard is finished *or* deferred, so "あとで設定する" is
 *    respected across restarts and does not nag.
 *  - the provider list, as a backstop. A flag that says "done" while no provider
 *    exists is a dead end — the Composer's send button needs a model, and the
 *    only thing that produces one is a provider.
 *
 * The forced flag exists so Settings can reopen the wizard on an installation
 * that already has providers, which is exactly the case the provider backstop
 * would otherwise veto.
 *
 * `ready` is separate from `show` because the shell must not flash in front of
 * the wizard: `useProviders` starts pending, and treating "no data yet" as "no
 * providers" would show the wizard to everyone on every cold start.
 */
export interface OnboardingGate {
  /** Wait for the provider list and the settings row before deciding. */
  ready: boolean;
  show: boolean;
  /** Finish or defer: persisted, so it does not come back next launch. */
  complete: () => void;
  /** Reopen from Settings. Session-only; see `onboardingForcedOpen`. */
  reopen: () => void;
}

export function useOnboardingGate(): OnboardingGate {
  const { data: settings, settled } = useSettingsForm();
  const providers = useProviders();
  const forcedOpen = useUi((s) => s.onboardingForcedOpen);
  const setForcedOpen = useUi((s) => s.setOnboardingForcedOpen);
  const applySettings = useApplySettings();

  const ready = settled && !providers.isPending;
  const configured = (providers.data?.length ?? 0) > 0;
  const flagged = settings.onboarding_completed === true;

  const show = ready && (forcedOpen || (!flagged && !configured));

  const complete = useCallback(() => {
    setForcedOpen(false);
    void applySettings({ onboarding_completed: true });
  }, [applySettings, setForcedOpen]);

  const reopen = useCallback(() => setForcedOpen(true), [setForcedOpen]);

  return { ready, show, complete, reopen };
}
