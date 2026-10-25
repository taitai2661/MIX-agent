/// Reusable pane-level error boundary with retry-reset.
///
/// `main.tsx` only guards the whole tree: a render throw in any pane blanked
/// the window. Panes that own fallible render paths (transcript markdown,
/// composer menus, settings forms) wrap themselves so one bad row degrades to
/// a card with a retry instead of taking the shell down. `resetKeys` clear
/// the error when the inputs change (e.g. switching threads after a message
/// with hostile content crashed the transcript).

import { Component, type ReactNode } from "react";
import { t } from "@/lib/i18n";
import { Alert } from "./ui/alert";
import { Button } from "./ui/button";

export function PaneErrorBoundary({
  children,
  name,
  resetKeys = [],
}: {
  children: ReactNode;
  /** Pane name for the console report (e.g. "Transcript"). */
  name: string;
  /** Values that clear a recorded error when they change. */
  resetKeys?: readonly unknown[];
}) {
  return (
    <PaneBoundary name={name} resetKeys={resetKeys}>
      {children}
    </PaneBoundary>
  );
}

interface BoundaryProps {
  children: ReactNode;
  name: string;
  resetKeys: readonly unknown[];
}

interface BoundaryState {
  error: Error | null;
}

function keysEqual(a: readonly unknown[], b: readonly unknown[]): boolean {
  return a.length === b.length && a.every((v, i) => Object.is(v, b[i]));
}

class PaneBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): BoundaryState {
    return { error };
  }

  componentDidCatch(error: Error): void {
    console.error(
      `MIX-agent ${this.props.name} pane crashed while rendering`,
      error,
    );
  }

  componentDidUpdate(prevProps: BoundaryProps): void {
    if (
      this.state.error &&
      !keysEqual(prevProps.resetKeys, this.props.resetKeys)
    ) {
      // The inputs moved on (new thread, new run): give the pane another go
      // instead of pinning the stale crash.
      this.setState({ error: null });
    }
  }

  private retry = () => this.setState({ error: null });

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div
        className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center"
        role="status"
      >
        <Alert tone="danger" className="max-w-md">
          {error.message || t("エラー")}
        </Alert>
        <Button variant="default" size="sm" onClick={this.retry}>
          {t("再試行")}
        </Button>
      </div>
    );
  }
}
