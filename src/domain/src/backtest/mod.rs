//! 予想精度バックテストの指標集計（純粋ロジック・IO なし）。
//!
//! 各評価レースの「予測と実着順の突合結果」（[`RaceEvaluation`]）を受け取り、
//! 的中率（単勝・連対・複勝）・想定回収率・キャリブレーション指標（Brier / LogLoss）を
//! [`BacktestReport`] に集計する。確率推定の再現やデータ取得は use-case 層が担い、本モジュールは
//! 集計のみを行う（設計書 `docs/specifications/backtest.md` 参照）。

/// LogLoss で `ln(0)` を避けるための確率クランプ幅。`p` を `[EPS, 1-EPS]` に収める。
const LOG_LOSS_EPS: f64 = 1e-15;

/// 1 レース 1 賭けの想定賭け金（円）。トップ選好馬の単勝に固定額を賭ける。
///
/// `odds` は単勝確定オッズ（払戻倍率）で、的中時の払戻は `odds × STAKE_PER_RACE`。JRA 実払戻の
/// 端数処理（100 円あたり 10 円未満切り捨て）は行わない理論値であり、実払戻とは厳密には一致しない。
const STAKE_PER_RACE: f64 = 100.0;

/// 1 レース分の予測と実着の突合結果（集計の純粋入力）。
#[derive(Debug, Clone)]
pub struct RaceEvaluation {
    /// 全出走馬の `(win_prob, その馬が 1 着か)`。Brier / LogLoss に使う。
    pub win_outcomes: Vec<(f64, bool)>,
    /// トップ選好馬（`win_prob` 最大、同値は馬番昇順）の着順。
    /// 除外・失格等で着順が無い場合は `None`（非的中扱い）。
    pub top_pick_position: Option<u32>,
    /// トップ選好馬の単勝確定オッズ。`None` なら回収率の母数外。
    pub top_pick_odds: Option<f64>,
}

/// バックテストの集計結果。
#[derive(Debug, Clone, PartialEq)]
pub struct BacktestReport {
    /// 突合できた評価レース数（的中率の母数）。
    pub races_evaluated: u32,
    /// 単勝的中率（トップ選好馬が 1 着）。
    pub win_hit_rate: f64,
    /// 連対的中率（トップ選好馬が 2 着以内）。
    pub place_hit_rate: f64,
    /// 複勝的中率（トップ選好馬が 3 着以内）。
    pub show_hit_rate: f64,
    /// 想定回収率（Σ payout / Σ stake）。オッズ取得レースが 0 件なら `None`。
    pub payout_rate: Option<f64>,
    /// 回収率の母数（オッズが取得できたレース数）。
    pub payout_races: u32,
    /// Brier スコア（win, 小さいほど良い）。
    pub brier: f64,
    /// 対数損失（win, 小さいほど良い）。
    pub log_loss: f64,
}

impl BacktestReport {
    /// 評価レースが 0 件のときの空レポート（指標は 0 / 回収率は `None`）。
    fn empty() -> Self {
        Self {
            races_evaluated: 0,
            win_hit_rate: 0.0,
            place_hit_rate: 0.0,
            show_hit_rate: 0.0,
            payout_rate: None,
            payout_races: 0,
            brier: 0.0,
            log_loss: 0.0,
        }
    }
}

/// 評価レース集合から [`BacktestReport`] を集計する。
///
/// 的中率の母数は `races.len()`（突合できたレース）。トップ選好馬の着順が `None` の
/// レースは全的中率で非的中として数える。回収率は `top_pick_odds` がある レースのみを母数に、
/// トップ選好馬が 1 着なら `odds × STAKE_PER_RACE` を払戻として計上する。Brier / LogLoss は
/// 全レースの全馬エントリ（`win_outcomes`）を母数に算出する。
pub fn evaluate(races: &[RaceEvaluation]) -> BacktestReport {
    if races.is_empty() {
        return BacktestReport::empty();
    }

    let n = races.len() as f64;
    let mut win_hits = 0u32;
    let mut place_hits = 0u32;
    let mut show_hits = 0u32;

    let mut payout_races = 0u32;
    let mut total_stake = 0.0f64;
    let mut total_payout = 0.0f64;

    let mut outcome_count = 0u32;
    let mut brier_sum = 0.0f64;
    let mut log_loss_sum = 0.0f64;

    for race in races {
        if let Some(pos) = race.top_pick_position {
            if pos == 1 {
                win_hits += 1;
            }
            if pos <= 2 {
                place_hits += 1;
            }
            if pos <= 3 {
                show_hits += 1;
            }
        }

        if let Some(odds) = race.top_pick_odds {
            payout_races += 1;
            total_stake += STAKE_PER_RACE;
            if race.top_pick_position == Some(1) {
                total_payout += odds * STAKE_PER_RACE;
            }
        }

        for &(prob, won) in &race.win_outcomes {
            let y = if won { 1.0 } else { 0.0 };
            brier_sum += (prob - y).powi(2);

            let p = prob.clamp(LOG_LOSS_EPS, 1.0 - LOG_LOSS_EPS);
            log_loss_sum += -(y * p.ln() + (1.0 - y) * (1.0 - p).ln());

            outcome_count += 1;
        }
    }

    let payout_rate = if payout_races > 0 {
        Some(total_payout / total_stake)
    } else {
        None
    };

    let (brier, log_loss) = if outcome_count > 0 {
        let c = outcome_count as f64;
        (brier_sum / c, log_loss_sum / c)
    } else {
        (0.0, 0.0)
    };

    BacktestReport {
        races_evaluated: races.len() as u32,
        win_hit_rate: win_hits as f64 / n,
        place_hit_rate: place_hits as f64 / n,
        show_hit_rate: show_hits as f64 / n,
        payout_rate,
        payout_races,
        brier,
        log_loss,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn empty_returns_zeroed_report() {
        let report = evaluate(&[]);
        assert_eq!(report, BacktestReport::empty());
        assert_eq!(report.races_evaluated, 0);
        assert!(report.payout_rate.is_none());
    }

    #[test]
    fn two_races_known_values() {
        let races = vec![
            RaceEvaluation {
                win_outcomes: vec![(0.5, true), (0.5, false)],
                top_pick_position: Some(1),
                top_pick_odds: Some(2.0),
            },
            RaceEvaluation {
                win_outcomes: vec![(0.6, false), (0.4, true)],
                top_pick_position: Some(3),
                top_pick_odds: Some(5.0),
            },
        ];
        let r = evaluate(&races);

        assert_eq!(r.races_evaluated, 2);
        approx(r.win_hit_rate, 0.5); // race1 のみ 1 着
        approx(r.place_hit_rate, 0.5); // race1 のみ 2 着以内
        approx(r.show_hit_rate, 1.0); // race1(1着) + race2(3着)

        // 回収率: stake=200, payout=race1 のみ 2.0*100=200 → 1.0
        assert_eq!(r.payout_races, 2);
        approx(r.payout_rate.unwrap(), 1.0);

        // Brier = (0.25+0.25+0.36+0.36)/4
        approx(r.brier, (0.25 + 0.25 + 0.36 + 0.36) / 4.0);

        // LogLoss = (-ln0.5 -ln0.5 -ln0.4 -ln0.4)/4
        let expected_ll = (-(0.5f64).ln() - (0.5f64).ln() - (0.4f64).ln() - (0.4f64).ln()) / 4.0;
        approx(r.log_loss, expected_ll);
    }

    #[test]
    fn payout_rate_none_when_no_odds() {
        let races = vec![RaceEvaluation {
            win_outcomes: vec![(0.7, true), (0.3, false)],
            top_pick_position: Some(1),
            top_pick_odds: None,
        }];
        let r = evaluate(&races);
        assert_eq!(r.payout_races, 0);
        assert!(r.payout_rate.is_none());
        // 的中率・キャリブレーションは算出される
        approx(r.win_hit_rate, 1.0);
    }

    #[test]
    fn top_pick_none_position_counts_as_miss() {
        let races = vec![RaceEvaluation {
            win_outcomes: vec![(0.4, false), (0.6, false)],
            top_pick_position: None, // 除外・失格等
            top_pick_odds: Some(3.0),
        }];
        let r = evaluate(&races);
        approx(r.win_hit_rate, 0.0);
        approx(r.place_hit_rate, 0.0);
        approx(r.show_hit_rate, 0.0);
        // 賭けは成立・非的中なので回収率 0、母数 1
        assert_eq!(r.payout_races, 1);
        approx(r.payout_rate.unwrap(), 0.0);
    }

    #[test]
    fn zero_prob_winner_keeps_log_loss_finite() {
        // スタッツ希薄でスコア 0 → win_prob=0 の馬が実際に勝ったケース
        let races = vec![RaceEvaluation {
            win_outcomes: vec![(0.0, true), (1.0, false)],
            top_pick_position: Some(5),
            top_pick_odds: None,
        }];
        let r = evaluate(&races);
        assert!(r.log_loss.is_finite(), "log_loss must be finite");
        assert!(r.brier.is_finite());
        // ε クランプにより -ln(EPS) 近傍の大きな有限値
        assert!(r.log_loss > 0.0);
    }

    #[test]
    fn hit_rates_respect_inclusion() {
        // トップ選好馬固定のため 単勝 ≤ 連対 ≤ 複勝 が常に成立
        let races = vec![
            RaceEvaluation {
                win_outcomes: vec![(1.0, true)],
                top_pick_position: Some(1),
                top_pick_odds: None,
            },
            RaceEvaluation {
                win_outcomes: vec![(1.0, false)],
                top_pick_position: Some(2),
                top_pick_odds: None,
            },
            RaceEvaluation {
                win_outcomes: vec![(1.0, false)],
                top_pick_position: Some(3),
                top_pick_odds: None,
            },
        ];
        let r = evaluate(&races);
        assert!(r.win_hit_rate <= r.place_hit_rate);
        assert!(r.place_hit_rate <= r.show_hit_rate);
        approx(r.win_hit_rate, 1.0 / 3.0);
        approx(r.place_hit_rate, 2.0 / 3.0);
        approx(r.show_hit_rate, 1.0);
    }
}
