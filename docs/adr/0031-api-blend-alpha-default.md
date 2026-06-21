# ADR 0031: API の blend_alpha 既定を本番ブレンド（α=0.3）に変更 (Issue #200)

## ステータス
承認済み

## コンテキスト

`GET /api/races/{id}/prediction` と `GET /api/races/{id}/recommendations` の
`blend_alpha` クエリパラメータを省略した場合、現状は素のモデル確率（α 未設定 = 市場オッズ不使用）
を返す。

一方、CLI の `paddock-predict`・ライブ EV (`paddock-analyze predict`) はいずれも α=0.3 を
本番パラメータとして使用しており、ADR 0027 でも「精度の主レバーは市場オッズブレンドである」と
確認済み。Web SPA (#34) は PR #202 で `blend_alpha=0.3` をクライアント側でハードコードする
暫定対処を取っているが、これは「API の既定がおかしい」という根本問題を先送りしたものに過ぎない。

## 決定

`GET /api/races/{id}/prediction` および `GET /api/races/{id}/recommendations` において、
`blend_alpha` が省略された場合のデフォルト値を **0.3**（本番ブレンド係数）にする。

- ハンドラ内で `PRODUCTION_BLEND_ALPHA: f64 = 0.3` 定数を定義し、`None` の場合に
  `Some(PRODUCTION_BLEND_ALPHA)` へフォールバックする。
- 明示指定（`blend_alpha=0.0`〜`1.0`）は引き続き尊重され、素モデル(`blend_alpha=1.0`)への
  アクセスも可能。
- SPA 側の暫定ハードコード (`PREDICT_BLEND_ALPHA = 0.3`) は削除し、API の既定に委ねる。
- OpenAPI 仕様（ドキュメントコメント）を更新して "未指定なら本番ブレンド α=0.3 を使用" と
  明示する。

## 理由

- **CLI・SPA・ライブ EV の全コンシューマが α=0.3 を使う**: 省略時のデフォルトを揃えないと
  呼び出しごとに結果が変わり、本命が食い違う（PR #202 の背景）。
- **素モデルは開発・検証時の特殊ケース**: `alpha=1.0` を明示すれば引き続きアクセスできる。
  省略を「素モデルを望む」と解釈するのは不自然。
- **クライアント側ハードコードは保守負担**: 新しいクライアントが `blend_alpha` を知らずに
  呼ぶと本番と異なる結果を返す。サーバ側で正しいデフォルトを持つべき。

## 影響

- 後方互換の破壊: **あり**（`blend_alpha` 省略時の返却値が変わる）。
  現時点の既知コンシューマは SPA（`RaceDetail.tsx`）・CLI（`paddock-predict`）・
  ライブ EV（`paddock-analyze predict`）の 3 つで、いずれも α=0.3 相当の動作を前提としている。
  `blend_alpha` を省略するだけの素の呼び出しは想定コンシューマに存在しない。
- `blend_alpha=1.0` を明示すれば旧来の素モデル挙動を再現できる。
  既知コンシューマ以外（将来のクライアント含む）が旧挙動を必要とする場合も同様に `blend_alpha=1.0` を指定すること。
- SPA 側の暫定回避コード（PR #202 で追加した `PREDICT_BLEND_ALPHA = 0.3`）が撤廃可能になり、
  新規クライアントが `blend_alpha` を意識せずに呼んでも本番挙動が得られる。
