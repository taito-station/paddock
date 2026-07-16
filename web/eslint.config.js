import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

// Vite 公式 React + TS テンプレート相当の flat config（#413）。
// 目的は react-hooks の依存配列検査（exhaustive-deps）と rules-of-hooks を CI ゲート化し、
// 退行検知を人力レビュー依存から外すこと。
//
// react-hooks は v7 を使うが、recommended プリセットが同梱する React Compiler 系ルール
// （static-components / use-memo / immutability 等）は有効化しない。Compiler 採用は別途 ADR を
// 要する独立判断であり、本 issue（依存配列検査の導入）に混ぜるとスコープが広がるため、
// テンプレート由来の 2 ルールのみを明示的に error 化する。
export default tseslint.config(
  // ビルド生成物と自動生成の API 型（openapi-typescript 出力）は検査対象外。
  { ignores: ["dist", "src/api/schema.d.ts"] },
  {
    files: ["**/*.{ts,tsx}"],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      // Vite テンプレートでは warn だが、本 issue の主目的なので error にして CI で確実に止める。
      "react-hooks/exhaustive-deps": "error",
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
    },
  },
);
