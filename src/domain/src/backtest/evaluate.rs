//! 評価レース集合から [`BacktestReport`] を集計するトップレベル関数。

use super::metrics::{RELIABILITY_BINS, calibration, reliability};
use super::model::{BacktestReport, RaceEvaluation, Top3RankDistribution};
use super::segments::{field_size_segments, popularity_segments, surface_segments};

/// 1 レース 1 賭けの想定賭け金（円）。トップ選好馬の単勝に固定額を賭ける。
///
/// `odds` は単勝確定オッズ（払戻倍率）で、的中時の払戻は `odds × STAKE_PER_RACE`。JRA 実払戻の
/// 端数処理（100 円あたり 10 円未満切り捨て）は行わない理論値であり、実払戻とは厳密には一致しない。
const STAKE_PER_RACE: f64 = 100.0;

/// 評価レース集合から [`BacktestReport`] を集計する。
///
/// 的中率の母数は `races.len()`（突合できたレース）。トップ選好馬の着順が `None` の
/// レースは全的中率で非的中として数える。回収率は `top_pick_odds` がある レースのみを母数に、
/// トップ選好馬が 1 着なら `odds × STAKE_PER_RACE` を払戻として計上する。校正指標（Brier /
/// LogLoss）は単勝・連対・複勝それぞれの全馬エントリを母数に算出し、reliability 曲線は単勝確率に
/// ついて、人気帯・頭数帯・馬場(芝/ダート)別のセグメントも併せて出す。
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

    // 全エントリの校正用ペア。
    let mut win_pairs: Vec<(f64, bool)> = Vec::new();
    let mut place_pairs: Vec<(f64, bool)> = Vec::new();
    let mut show_pairs: Vec<(f64, bool)> = Vec::new();

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

        for h in &race.horses {
            win_pairs.push((h.win_prob, h.won()));
            place_pairs.push((h.place_prob, h.placed()));
            show_pairs.push((h.show_prob, h.showed()));
        }
    }

    let payout_rate = if payout_races > 0 {
        Some(total_payout / total_stake)
    } else {
        None
    };

    let win_calibration = calibration(&win_pairs);

    BacktestReport {
        top3_rank_distribution: top3_rank_distribution(races),
        place_reliability: reliability(&place_pairs, RELIABILITY_BINS),
        show_reliability: reliability(&show_pairs, RELIABILITY_BINS),
        races_evaluated: races.len() as u32,
        win_hit_rate: win_hits as f64 / n,
        place_hit_rate: place_hits as f64 / n,
        show_hit_rate: show_hits as f64 / n,
        payout_rate,
        payout_races,
        brier: win_calibration.brier,
        log_loss: win_calibration.log_loss,
        place_calibration: calibration(&place_pairs),
        show_calibration: calibration(&show_pairs),
        win_reliability: reliability(&win_pairs, RELIABILITY_BINS),
        by_field_size: field_size_segments(races),
        by_popularity: popularity_segments(races),
        by_surface: surface_segments(races),
        // 買い目（curated）の校正・回収率は買い目単位の別入力（exotic_segments）で埋める（#121）。
        by_exotic: Vec::new(),
    }
}

/// 3 着以内入線馬のモデル `show_prob` 順位分布を集計する（#258）。
///
/// 各レースで全出走馬を `show_prob` 降順（同値は元順で安定）に並べ、実際に 3 着以内へ来た馬が
/// モデルで何位だったかを 1-3 / 4-6 / 7+ にバケットする。`model_rank_7_plus` が多いほど、モデルは
/// 複勝圏に飛び込む人気薄を下位に沈めて取りこぼしている（複勝圏の過小評価）。
fn top3_rank_distribution(races: &[RaceEvaluation]) -> Top3RankDistribution {
    let mut dist = Top3RankDistribution::default();
    for race in races {
        // show_prob 降順インデックス（slice::sort_by は安定ソートなので同値は元順）。
        let mut order: Vec<usize> = (0..race.horses.len()).collect();
        order.sort_by(|&a, &b| {
            race.horses[b]
                .show_prob
                .partial_cmp(&race.horses[a].show_prob)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // 馬 index → モデル順位（1 始まり）の逆引き。
        let mut rank_of = vec![0u32; race.horses.len()];
        for (r, &h) in order.iter().enumerate() {
            rank_of[h] = (r + 1) as u32;
        }
        for (h, horse) in race.horses.iter().enumerate() {
            if horse.showed() {
                dist.finishers += 1;
                match rank_of[h] {
                    1..=3 => dist.model_rank_1_3 += 1,
                    4..=6 => dist.model_rank_4_6 += 1,
                    _ => dist.model_rank_7_plus += 1,
                }
            }
        }
    }
    dist
}
