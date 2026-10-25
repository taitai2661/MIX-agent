import { useState } from "react";
import { AlertTriangle, ChevronDown, RefreshCw } from "lucide-react";

interface ErrorBoxProps {
  error: unknown;
  onRetry?: () => void;
  showDetails?: boolean;
}

export function ErrorBox({ error, onRetry, showDetails = true }: ErrorBoxProps) {
  const [expanded, setExpanded] = useState(false);
  if (!error) return null;

  const message = error instanceof Error ? error.message : String(error);
  const details = error instanceof Error ? error.stack : undefined;

  return (
    <div role="alert" className="error-box">
      <div className="error-box-header">
        <AlertTriangle size={16} />
        <span>{message}</span>
        {onRetry && (
          <button type="button" className="error-retry" onClick={onRetry}>
            <RefreshCw size={12} />
            再試行
          </button>
        )}
        {showDetails && details && (
          <button
            type="button"
            className="error-expand"
            aria-expanded={expanded}
            onClick={() => setExpanded(!expanded)}
          >
            <ChevronDown size={12} className={expanded ? "expanded" : ""} />
          </button>
        )}
      </div>
      {expanded && details && (
        <pre className="error-details">{details}</pre>
      )}
    </div>
  );
}
