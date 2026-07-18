import { Component } from "react";
import type { ErrorInfo, ReactNode } from "react";

type ErrorBoundaryProps = {
  children: ReactNode;
};

type ErrorBoundaryState = {
  hasError: boolean;
};

// ルート直下で描画例外を捕捉し、白画面（無反応の空画面）を防ぐ最後の砦（#417）。
// React の error boundary は class component でしか実装できないため、ここだけ class を使う。
export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  state: ErrorBoundaryState = { hasError: false };

  // 例外発生時に fallback 描画へ切り替えるだけの純関数（副作用は持たせない）。
  static getDerivedStateFromError(): ErrorBoundaryState {
    return { hasError: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    // ログは開発者向け。UI には汎用文言のみ出し、詳細はコンソールに残す。
    console.error("ErrorBoundary caught:", error, info.componentStack);
  }

  render(): ReactNode {
    if (this.state.hasError) {
      return (
        <div className="error-boundary" role="alert">
          <h1>表示中に問題が発生しました</h1>
          <p>
            画面の描画に失敗しました。ページを再読み込みしてください。
            繰り返す場合は時間をおいて再度お試しください。
          </p>
          <button
            type="button"
            className="error-boundary-reload"
            onClick={() => window.location.reload()}
          >
            再読み込み
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
