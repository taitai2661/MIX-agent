import { Component, type ErrorInfo, type ReactNode } from "react";
import { t } from "@/app/i18n";
import { Button } from "@/components/button";

interface Props {
  children: ReactNode;
}
interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("ErrorBoundary", error, info);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="auth">
          <section className="auth-card error-fallback" role="alert">
            <h2>{t("エラーが発生しました")}</h2>
            <p>
              {t(
                "予期しないエラーが発生しました。以下のボタンで再読み込みしてください。",
              )}
            </p>
            <div className="form-actions">
              <Button
                onClick={() => this.setState({ hasError: false, error: null })}
              >
                {t("再試行")}
              </Button>
              <Button
                variant="outline"
                onClick={() => window.location.reload()}
              >
                {t("再読み込み")}
              </Button>
            </div>
            {this.state.error?.message && (
              <details>
                <summary>{t("エラーの詳細")}</summary>
                <pre>{this.state.error.message}</pre>
              </details>
            )}
          </section>
        </div>
      );
    }
    return this.props.children;
  }
}
