import { describe, expect, it } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

// vitest は node 環境で DOM を持たないため、render は行わず fallback 切替の要である
// 純粋な static メソッドのみ検証する（コンポーネント render テストは scope 過剰）。
describe("ErrorBoundary.getDerivedStateFromError", () => {
  it("例外時に hasError=true の state を返す", () => {
    expect(ErrorBoundary.getDerivedStateFromError()).toEqual({
      hasError: true,
    });
  });
});
