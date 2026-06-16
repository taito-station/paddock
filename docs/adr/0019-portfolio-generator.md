# ADR 0019: 予算内・軸流しポートフォリオ生成器 (Issue #122 PR2)

## ステータス
提案中

## コンテキスト
predict の買い目出力は `select_bets`（ADR 0003 / #121 curation 済み）の EV 羅列で、
「全正 EV 組合せをフラットに並べる」ため、そのままでは買えない（#122）。実際の買い方
（買い方メモ: 軸＝本命を外さない・相手広く・保険のワイド・100 円単位）が encode されておらず、
PR1（#138）の戦略評価ハーネスでも買い方次第で回収率が大きく変わる（本命単勝のみ 51.7% 等）ことが
定量化された。predict 本番に「予算内の軸流しポートフォリオ」を出力する生成器が必要。

`HorseProbability`（確率推定）、`RaceOdds`、単一レース収支シミュレータ `simulation::simulate`
（ADR 由来の `EvReport`）は既に Domain 層にある。

## 決定

1. **Domain 層に `portfolio` モジュールを新設する。**
   `src/domain/src/portfolio/mod.rs` に `PortfolioConfig` / `PortfolioBet` / `Portfolio` 型と
   `build_portfolio` 純粋関数を置く（IO なし）。`select_bets` は backtest 用に存続させる。

2. **軸流し（軸 1 頭ながし）を生成する。**
   軸 = `win_prob` 最大の馬（予想本命）、相手 = 次点 `partners` 頭。馬連・ワイドは軸-相手の K 点、
   三連複は軸＋相手 2 頭（C(K,2) 点）。ワイドを保険として常に含める。

3. **予算は per-race（`--race-budget`）で受け、100 円単位で配分する。**
   `alloc` 重み (馬連:ワイド:三連複) で券種へ予算を割り、券種内は 100 円単位均等配分。賄えない端数は
   買わない（PR1 `strategy_eval.distribute` と同じ流儀）。実上限は `min(race_budget, 残高)`。
   券種割当て段階でも 100 円単位 floor を取るため、配分・点数次第で予算の一部（最大で数百円規模）が
   恒常的に未消化になりうる（消化率を上げる余り再配分は将来課題）。

4. **期待値は `simulate` に委譲する。**
   ポートフォリオ全体の期待回収率・的中確率、および各脚の EV 倍率（ワイドのように的中確率の
   閉形式が無い券種を含む）を着順列挙の収支シミュレータで正確に算出する。
   **オッズ未取得の脚は EV 評価から除外する**（払戻を見積もれず、`odds=0` で混ぜると的中 0 の stake が
   ROI 分母を膨らませ過小評価になるため）。よってポートフォリオ ROI は「オッズ取得済みの脚」についての
   期待回収率であり、未取得脚がある場合は predict 出力にその旨を注記する。

5. **predict 本番の買い目推奨を軸流しポートフォリオに置き換える。**
   `apps/predict/src/session.rs` の `select_bets`＋`recommended_amounts`（Kelly 比例配分）を
   `build_portfolio` に置換。購入(y/e/s)・DB 記録・精算フローは生成器の買い目に乗せる。
   `recommended_amounts` は役目を終えたため削除する。

6. **ワイドのライブオッズ取得を追加する。**
   保険ワイドをオッズ/EV 込みで扱うため、`odds-scraper`（production の `UreqOddsScraper`）に
   ワイドページの取得・パース（`parse_wide`, 帯 low..high）を追加する。永続化（`race_odds`）は対応済み。

## 理由
- Domain の純粋関数として置くことで use-case/apps から依存なく呼べ、PR1 ハーネスでも検証しやすい。
- 軸流し＋保険ワイドは買い方メモを直接 encode し、EV 羅列より「そのまま買える」出力になる。
- EV を `simulate` に一本化することで、券種ごとの的中確率近似を重複実装せず正確さを担保する。
- 既定値（相手 5 頭・配分 1:1:1）は固定だが、PR1 の `strategy_eval.py` で相手頭数・配分の感度を
  測って後追い調整できる（「効果が無ければ採用しない」）。

## 影響・代替案
- 単勝/馬単/三連単は軸流しの対象外（predict 出力から外れる）。買い方メモが馬連/ワイド/三連複の
  ながしを中心とするため。backtest の `select_bets` には影響しない。
- 人気軸（market favorite）切替・配分の動的最適化は将来課題（既定は本命軸・固定配分）。
- 確率/EV の校正は #121 側で別途扱う（本 ADR は買い方＝馬券構成のみ）。
