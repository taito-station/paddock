# netkeiba の未発売番兵値（#621）

## `netkeiba_sentinels.txt`

netkeiba が「未発売・該当なしの組み合わせ」に入れる番兵値（1 行 1 値）。**払戻倍率ではない**ので、
オッズとして採用してはいけない。

このファイルは Rust と Python の**共通の正本**。同じ 3 値を両言語が別々に持つと、片方だけ更新して
静かにズレる（#587 の見出し契約と同型の事故）。

- Rust: `src/domain/src/odds/odds_value.rs` の `NETKEIBA_SENTINELS` と、テスト
  `sentinel_list_matches_the_shared_golden` が `include_str!` で突き合わせる
  （`#[cfg(test)]` 内なので、ファイルが消えると落ちるのは**テストビルド**）
- Python: `scripts/predict-check/odds_guard.py` がこのファイルを読んで集合を作る
  （`test_odds_guard.py` がパスと内容を張る）

## 値の出所（2026-08 時点の実測）

| 券種 | 番兵値 | 備考 |
|---|---|---|
| ワイド | `9999.9` | 相方が `odds_high=0.0` になるため、従来は**下限違反として偶然弾かれていた** |
| 馬連 / 馬単 / 三連複 | `99999.9` | ここが素通りして EV を壊していた（#621 の実害） |
| 三連単 | `999999.9` | DB 実測で 32,973 行 |

## なぜ「上限」ではなく特定値の除外か

三連単には `111971.9` / `200886.6` のような**正当な高配当が実在する**（DB 実測）。上限方式は
こうした大穴を殺すが、番兵は固定値なので特定値の除外なら誤爆しない。

値を足すときは **3 か所**を同じ PR で更新すること: このファイル / Rust の `NETKEIBA_SENTINELS` /
`scripts/predict-check/test_odds_guard.py` の期待タプル。どれか 1 つを忘れれば Rust か Python の
テストが落ちる。

**このファイルは `testdata/` ではない。** Python の `odds_guard.py` が **import 時に読む本番依存**で、
消すと 4 本の解析スクリプトが起動できなくなる。
