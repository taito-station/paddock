/// <reference types="vite/client" />
import { describe, expect, it } from "vitest";
// Vite の ?raw で CSS 本文を文字列取得する（vitest も同じ変換経路）。node:fs を避け
// ブラウザ環境の型（@types/node 非導入）のまま自己完結させる。
import cssText from "./styles.css?raw";

// 盤の色は :root のトークン（--bg 等）で一元管理する（#374）。:root の外に生の hex
// リテラルを増やさないための再発ガード（#385）。stylelint 未導入のため簡易テストで
// `color-no-hex` 相当を代替する（外部ツールを足さず自己完結する方針）。
//
// 意図的な例外（allowlist）は styles.css ヘッダの但し書きに準ずる:
//   - #fff … 純白は意味名が立たないため生値のまま残す。
// text-shadow の rgba() は hex ではないため元々この検査に掛からない。

// :root ブロックとブロックコメントを除いた本体に残る hex を洗い出す。
// コメントを先に除くのは、コメント中の Issue 参照（"#374" 等）を誤検出しないため。
function hexLiteralsOutsideRoot(css: string): string[] {
  const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
  // :root { ... } を除去（CSS のトークン定義は入れ子の中括弧を持たない前提で [^}]* で足りる）。
  const withoutRoot = withoutComments.replace(/:root\s*\{[^}]*\}/g, "");
  return withoutRoot.match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];
}

// styles.css ヘッダの但し書きが認める例外どおり #fff のみ（純白は意味名が立たない）。
// 現状 #ffffff 表記は無く、増やす動機も無いので文書と一致させ 1 エントリに絞る（YAGNI）。
const ALLOWLIST = new Set(["#fff"]);

describe("styles.css hex guard", () => {
  it(":root 外に allowlist 以外の生 hex を持たない", () => {
    const offenders = hexLiteralsOutsideRoot(cssText).filter(
      (h) => !ALLOWLIST.has(h.toLowerCase()),
    );
    expect(offenders).toEqual([]);
  });

  it("ヘルパは :root 外の非 allowlist hex を検出する（ガードの自己検証）", () => {
    const sample = `:root { --x: #123456; }\n.foo { color: #abcdef; background: #fff; }`;
    expect(hexLiteralsOutsideRoot(sample)).toEqual(["#abcdef", "#fff"]);
  });
});
