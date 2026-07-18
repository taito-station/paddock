import { describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { ReactElement } from "react";
import { ErrorBoundary } from "./ErrorBoundary";

// vitest は node 環境で DOM を持たないため mount（実描画）は行わない。代わりに
// render() をインスタンスメソッドとして直接呼び、返り値の要素構造だけを検証する
// （DOM 非依存で fallback 分岐・children パススルーの回帰を押さえる）。
describe("ErrorBoundary", () => {
  it("getDerivedStateFromError は例外時に hasError=true を返す", () => {
    expect(ErrorBoundary.getDerivedStateFromError()).toEqual({
      hasError: true,
    });
  });

  it("正常時は children をそのまま描画する", () => {
    const inst = new ErrorBoundary({ children: "child-node" });
    expect(inst.render()).toBe("child-node");
  });

  it("例外捕捉後は fallback 要素を描画する（children は出さない）", () => {
    const inst = new ErrorBoundary({ children: "child-node" });
    inst.state = { hasError: true };
    const el = inst.render();
    expect(isValidElement(el)).toBe(true);
    expect(
      (el as ReactElement<{ className?: string }>).props.className,
    ).toBe("error-boundary");
  });

  it("componentDidCatch は詳細を console.error に残す", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    const inst = new ErrorBoundary({ children: null });
    inst.componentDidCatch(new Error("boom"), { componentStack: "<Foo>" });
    expect(spy).toHaveBeenCalledOnce();
    spy.mockRestore();
  });
});
