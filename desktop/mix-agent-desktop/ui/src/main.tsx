import {
  Component,
  StrictMode,
  type ErrorInfo,
  type ReactNode,
} from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";
import { applyTheme, storedTheme } from "./lib/theme";
import { Button } from "./components/ui/button";
import "./styles/tokens.css";

// Apply the persisted theme before the first paint to avoid a flash.
applyTheme(storedTheme());

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("Missing #root element in index.html");
}

// Every pane reads and writes server state through react-query, so the client
// has to exist above <App /> — without it `useQueryClient()` throws during the
// first render and React tears the whole tree down, which surfaces as an empty
// window rather than an error.
const queryClient = new QueryClient();

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
    console.error("MIX-agent Desktop crashed while rendering", error, info.componentStack);
  }

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <h1 className="text-lg font-semibold text-fg">The interface failed to start</h1>
        <p className="max-w-lg text-sm text-danger">{error.message}</p>
        {error.stack && (
          <pre className="max-h-64 max-w-3xl overflow-auto rounded-lg bg-inset p-3 text-start font-mono text-2xs leading-4 text-muted">
            {error.stack}
          </pre>
        )}
        <Button variant="primary" onClick={() => window.location.reload()}>
          Reload
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
