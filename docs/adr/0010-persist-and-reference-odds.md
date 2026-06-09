# ADR 0010: オッズを永続化し predict/backtest から参照する (Issue #51)

## ステータス
承認済み

## コンテキスト
オッズの扱いは段階的に決めてきた:

- **ADR 0001（#10）**: `OddsScraper`（`scrape(&RaceId) -> RaceOdds`、都度スクレイプ・キャッシュなし）を実装。DB 永続化は別 Issue へ先送り。
- **ADR 0005（#25）**: predict にオッズを結線する際、案A（オンデマンド・都度スクレイプ）を採用し、スタブだった `Repository::find_race_odds` と `race_odds` 永続化（案B）を撤去。予想の再現や当時オッズ参照は将来 Issue へ先送りとした。
- **#28（PR #56）**: `race_odds` テーブル（`(race_id, bet_type, combination_key)` 主キー、`odds`/`odds_high`/`popularity`/`fetched_at`）と `save_race_odds`、`fetch-card` 経由の**単勝**取得・永続化を実装。

この結果、書き込み（fetch-card → race_odds）はあるのに読み出しが無く、predict は依然として毎回ライブスクレイプ、backtest は PDF 確定成績の単勝のみを使っており、保存済みオッズが活用されていなかった。本 Issue（#51）はこの読み出し側を仕上げ、ADR 0005 が先送りした「案B（永続化参照）」を **win+place に限って**採用する。

## 決定

1. **複勝(place)の取得・永続化を fetch-card に追加する。**
   netkeiba のオッズ API（`api_get_jra_odds.html?type=1`）は 1 レスポンスに単勝(`data.odds["1"]`)と複勝(`data.odds["2"]`)を同梱するため、`fetch_win_place_odds` として 1 回の取得で両方を返す。複勝は幅 odds なので `odds`=下限・`odds_high`=上限に保存する。

2. **読み出し `Repository::find_race_odds(race_id, as_of)` を新設する。**
   `race_odds` の `bet_type IN ('win','place')` をドメイン `RaceOdds` に再構成する。`as_of = Some(d)` のとき `date(fetched_at) <= d` のスナップショットに限定（backtest のリーク防止）、`None` で時刻制約なし（predict）。

3. **predict のオッズ取得を read-through に切り替える。**
   `OddsInteractor` に `Repository` を注入し、`race_odds()` を「保存済み(win+place)があれば返す → 無ければライブスクレイプし win+place を保存してフルのオッズを返す」とする。cache-miss 時に取得したフル（exotic 含む）はその回の買い目にそのまま使うが、保存・再参照は win+place に限る。

4. **backtest は当時オッズを優先し、無ければ PDF にフォールバックする。**
   トップ選好馬の回収率に使う単勝オッズを、`find_race_odds(race_id, Some(race.date))` の win があればそれ、無ければ従来どおり PDF 確定成績の `r.odds` を使う。

5. **組合せ券種（馬連・ワイド・3連複・3連単）の永続化はスコープ外**とし #38 に委ねる。`combination_key` 規約と netkeiba 取得は #38 で定義する。

## 理由
- 書き込みだけ存在して読み出しが無い状態を解消し、#28 で用意した `race_odds` を実際に活用する。予想の再現性（同一セッション再実行・resume で同じオッズ）と、当時オッズに基づく現実的なバックテスト回収率が得られる。
- ADR 0005 が撤去した案B を**全面復活ではなく win+place に限定**することで、未確定な exotic の `combination_key` 規約（#38）に踏み込まずに必要な価値（単複の再現・複勝の期待値計算）を取れる。
- backtest の PDF フォールバックにより、保存オッズが無い過去レースでも既存の長期バックテストが壊れない（移行コストゼロ）。
- read-through 方式は cache-miss 時もフルのオッズで買い目を出せるため、exotic 推奨の回帰を初回実行では起こさない。

## 影響
- `OddsInteractor<O>` が `OddsInteractor<O, R: Repository>` になり、predict の `setup.rs` で `SqliteRepository`（プール共有）を注入する。ADR 0001/0005 の「OddsInteractor は永続化を持たない」前提は本 ADR で更新される。
- 保存済み win+place を参照する resume・再実行では exotic 推奨が出ない（exotic は #38 で永続化されるまで cache-miss 時のフルスクレイプ回のみ）。
- スクレイプ由来の保存行は人気(`popularity`)を持たない（netkeiba の fetch-card 経由のみ人気が入る）。`popularity` は NULL 許容なので問題ない。
- backtest の回収率は、当時オッズが保存されたレースでは PDF 確定単勝ではなく当時オッズ基準になる（より現実的）。
- read-through の cache-hit 判定は「保存済み win/place が空でない」。単勝のみ保存された回（複勝未公開時など）は cache-hit して複勝を取り直さない。netkeiba/JRA は単複を同一レスポンスで返すため通常は両方そろうが、片側保存のエッジでは複勝が埋まらないことを許容する（必要になれば「両方そろうまで cache-miss 扱い」に強化する）。
- backtest の当時オッズ参照は `date(fetched_at)`(UTC) と `race.date`(JST 開催日) の粗い日付比較。TZ 境界（レース後の深夜取得は取りこぼし、当日内取得は同日付で通過）は厳密でないが、fetch-card/predict をレース前に走らせる運用前提で実害は小さい。

## 後日談（#38 で更新）
本 ADR の決定 5 と「影響」の win+place 限定は **#38 で解消**した。`OddsInteractor` は
スクレイプで得た**全券種**（馬連・ワイド・馬単・3連複・3連単を含む）を `race_odds` に保存し、
`find_race_odds` も全券種を読み戻すようになった。これにより resume・cache-hit 時も exotic 推奨が
出る。`combination_key` 規約はドメイン型（`Pair`/`OrderedPair`/`Triple`/`OrderedTriple`）の
`to_key`/`from_key` を単一情報源とする（昇順 `-` 連結、順序付きは `>` 連結）。スキーマ（汎用
`bet_type`/`combination_key`）はマイグレーション再設計なしでそのまま受けられた。`find_race_odds` は
SQL の `bet_type` フィルタを撤廃して全行を読むが、`BetType` で解釈できない未知ラベルの行は
読み飛ばす（撤廃前の「未知は無視」挙動を維持し、将来券種を書く新版 → 旧版で読む過渡期でも
predict/backtest を止めない）。なお組合せ券種は 1 レースで行数が増える（三連単は最大
18×17×16 = 4896 通り）が、`find_race_odds` は PK 先頭 `race_id` で 1 レースに絞って読むため
許容範囲とする。

## 関連
- ADR 0001（JRA オッズスクレイパー実装, #10）
- ADR 0005（predict にオッズを結線, #25）— 本 ADR が案B を限定的に復活させる
- Issue #28（race_odds テーブル・単勝永続化, PR #56）
- Issue #38（組合せ券種の combination_key 規約・取得）
- 設計書 `docs/specifications/netkeiba-datasource.md` / `predict-session.md` / `backtest.md`
