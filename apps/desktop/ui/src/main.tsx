import { Component, StrictMode, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";
import { t } from "./lib/i18n";
import { applyTheme, storedTheme } from "./lib/theme";
import { detectPlatformFromUa } from "./lib/platform";
import { pruneComposerDrafts } from "./lib/drafts";
import { Button } from "./components/ui/button";
import "./styles/tokens.css";

// Apply the persisted theme before the first paint to avoid a flash.
applyTheme(storedTheme());
// Publish a best-effort platform before React renders so shortcut labels and
// macOS-only settings are correct on the first paint. App refines this with
// `plugin-os`, which is authoritative inside the Tauri WebView.
document.documentElement.dataset.platform = detectPlatformFromUa(
  navigator.userAgent,
);
// Drafts expire after 30 days; drop stale records even if the user never opens
// the thread that owns them again.
pruneComposerDrafts();

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("Missing #root element in index.html");
}

// Every pane reads and writes server state through react-query, so the client
// has to exist above <App /> — without it `useQueryClient()` throws during the
// first render and React tears the whole tree down, which surfaces as an empty
// window rather than an error.
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // A command that does not exist (or a provider that is gone) is not
      // going to start answering on the fourth try; retrying three times only
      // delayed the error the UI could finally show.
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

interface BoundaryProps {
  children: ReactNode;
}

interface BoundaryState {
  error: Error | null;
}

/**
 * A render-time throw inside <App /> would otherwise blank the window with no
 * clue as to why. Surface the message and keep the WebView devtools useful.
 */
class AppErrorBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): BoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error(
      "MIX-agent Desktop crashed while rendering",
      error,
      info.componentStack,
    );
  }

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <h1 className="text-lg font-semibold text-fg">
          {t("インターフェースの起動に失敗しました")}
        </h1>
        <p className="max-w-lg text-sm text-danger">{error.message}</p>
        {error.stack && (
          <pre className="max-h-64 max-w-3xl overflow-auto rounded-lg bg-inset p-3 text-start font-mono text-xs leading-4 text-muted">
            {error.stack}
          </pre>
        )}
        <Button variant="primary" onClick={() => window.location.reload()}>
          {t("再読み込み")}
        </Button>
      </div>
    );
  }
}

createRoot(rootEl).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <AppErrorBoundary>
        <App />
      </AppErrorBoundary>
    </QueryClientProvider>
  </StrictMode>,
);
