import { Component, type ErrorInfo, type ReactNode } from "react";
import { t } from "@/app/i18n";

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
        <main className="auth">
          <h2>{t("エラーが発生しました")}</h2>
          <p>
            {t("予期しないエラーが発生しました。以下のボタンで再読み込みしてください。")}
          </p>
          <button onClick={() => window.location.reload()}>{t("再読み込み")}</button>
        </main>
      );
    }
    return this.props.children;
  }
}
