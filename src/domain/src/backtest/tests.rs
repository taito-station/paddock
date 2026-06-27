use super::*;
use crate::Surface;

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
}

/// テスト用の馬 outcome。win/place/show 確率と着順・人気を与える。
fn horse(win: f64, place: f64, show: f64, pos: Option<u32>, pop: Option<u32>) -> HorseOutcome {
    HorseOutcome {
        win_prob: win,
        place_prob: place,
        show_prob: show,
        finishing_position: pos,
        popularity: pop,
    }
}

/// win 確率と着順だけ指定する簡易版（place/show は win と同値、人気なし）。
fn win_horse(win: f64, pos: Option<u32>) -> HorseOutcome {
    horse(win, win, win, pos, None)
}

#[test]
fn empty_returns_zeroed_report() {
    let report = evaluate(&[]);
    assert_eq!(report, BacktestReport::empty());
    assert_eq!(report.races_evaluated, 0);
    assert!(report.payout_rate.is_none());
    assert!(report.win_reliability.is_empty());
    assert!(report.by_field_size.is_empty());
    assert!(report.by_popularity.is_empty());
}

#[test]
fn two_races_known_values() {
    let races = vec![
        RaceEvaluation {
            horses: vec![win_horse(0.5, Some(1)), win_horse(0.5, Some(2))],
            top_pick_position: Some(1),
            top_pick_odds: Some(2.0),
            surface: Surface::Turf,
        },
        RaceEvaluation {
            // トップ選好(0.6)は 3 着、勝ったのは 0.4 の馬。
            horses: vec![win_horse(0.6, Some(3)), win_horse(0.4, Some(1))],
            top_pick_position: Some(3),
            top_pick_odds: Some(5.0),
            surface: Surface::Turf,
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

    // Brier(win) = (0.25+0.25+0.36+0.36)/4
    approx(r.brier, (0.25 + 0.25 + 0.36 + 0.36) / 4.0);

    // LogLoss(win) = (-ln0.5 -ln0.5 -ln0.4 -ln0.4)/4
    let expected_ll = (-(0.5f64).ln() - (0.5f64).ln() - (0.4f64).ln() - (0.4f64).ln()) / 4.0;
    approx(r.log_loss, expected_ll);
}

#[test]
fn place_and_show_calibration_known_values() {
    // 1 レース・2 頭。place_prob/show_prob と着順から連対/複勝校正を検証する。
    // 馬A: 1 着 → placed=true, showed=true。馬B: 4 着 → placed=false, showed=false。
    let races = vec![RaceEvaluation {
        horses: vec![
            horse(0.5, 0.7, 0.8, Some(1), Some(1)),
            horse(0.5, 0.6, 0.7, Some(4), Some(2)),
        ],
        top_pick_position: Some(1),
        top_pick_odds: None,
        surface: Surface::Turf,
    }];
    let r = evaluate(&races);

    // place: (0.7,true),(0.6,false) → Brier=((0.3)^2+(0.6)^2)/2
    approx(
        r.place_calibration.brier,
        (0.3f64.powi(2) + 0.6f64.powi(2)) / 2.0,
    );
    let place_ll = (-(0.7f64).ln() - (1.0 - 0.6f64).ln()) / 2.0;
    approx(r.place_calibration.log_loss, place_ll);

    // show: (0.8,true),(0.7,false) → Brier=((0.2)^2+(0.7)^2)/2
    approx(
        r.show_calibration.brier,
        (0.2f64.powi(2) + 0.7f64.powi(2)) / 2.0,
    );
    let show_ll = (-(0.8f64).ln() - (1.0 - 0.7f64).ln()) / 2.0;
    approx(r.show_calibration.log_loss, show_ll);
}

#[test]
fn payout_rate_none_when_no_odds() {
    let races = vec![RaceEvaluation {
        horses: vec![win_horse(0.7, Some(1)), win_horse(0.3, Some(2))],
        top_pick_position: Some(1),
        top_pick_odds: None,
        surface: Surface::Turf,
    }];
    let r = evaluate(&races);
    assert_eq!(r.payout_races, 0);
    assert!(r.payout_rate.is_none());
    approx(r.win_hit_rate, 1.0);
}

#[test]
fn top_pick_none_position_counts_as_miss() {
    let races = vec![RaceEvaluation {
        horses: vec![win_horse(0.4, None), win_horse(0.6, None)],
        top_pick_position: None, // 除外・失格等
        top_pick_odds: Some(3.0),
        surface: Surface::Turf,
    }];
    let r = evaluate(&races);
    approx(r.win_hit_rate, 0.0);
    approx(r.place_hit_rate, 0.0);
    approx(r.show_hit_rate, 0.0);
    assert_eq!(r.payout_races, 1);
    approx(r.payout_rate.unwrap(), 0.0);
}

#[test]
fn zero_prob_winner_keeps_log_loss_finite() {
    let races = vec![RaceEvaluation {
        horses: vec![win_horse(0.0, Some(1)), win_horse(1.0, Some(5))],
        top_pick_position: Some(5),
        top_pick_odds: None,
        surface: Surface::Turf,
    }];
    let r = evaluate(&races);
    assert!(r.log_loss.is_finite(), "log_loss must be finite");
    assert!(r.brier.is_finite());
    assert!(r.log_loss > 0.0);
}

#[test]
fn hit_rates_respect_inclusion() {
    let races = vec![
        RaceEvaluation {
            horses: vec![win_horse(1.0, Some(1))],
            top_pick_position: Some(1),
            top_pick_odds: None,
            surface: Surface::Turf,
        },
        RaceEvaluation {
            horses: vec![win_horse(1.0, Some(2))],
            top_pick_position: Some(2),
            top_pick_odds: None,
            surface: Surface::Turf,
        },
        RaceEvaluation {
            horses: vec![win_horse(1.0, Some(3))],
            top_pick_position: Some(3),
            top_pick_odds: None,
            surface: Surface::Turf,
        },
    ];
    let r = evaluate(&races);
    assert!(r.win_hit_rate <= r.place_hit_rate);
    assert!(r.place_hit_rate <= r.show_hit_rate);
    approx(r.win_hit_rate, 1.0 / 3.0);
    approx(r.place_hit_rate, 2.0 / 3.0);
    approx(r.show_hit_rate, 1.0);
}

#[test]
fn reliability_bins_split_and_aggregate() {
    let pairs = [
        (0.05, false),
        (0.0, false),
        (0.95, true),
        (0.95, true),
        (1.0, true), // 上端は最終ビンへ
    ];
    let bins = reliability(&pairs, 10);
    assert_eq!(bins.len(), 10);

    // bin0 = [0.0,0.1): 0.05 と 0.0 の 2 件、勝ち 0。
    approx(bins[0].lower, 0.0);
    approx(bins[0].upper, 0.1);
    assert_eq!(bins[0].count, 2);
    approx(bins[0].mean_predicted, 0.025);
    approx(bins[0].observed_rate, 0.0);

    // 中間ビンは空。
    assert_eq!(bins[5].count, 0);
    approx(bins[5].mean_predicted, 0.0);
    approx(bins[5].observed_rate, 0.0);

    // bin9 = [0.9,1.0]: 0.95,0.95,1.0 の 3 件、全勝。
    assert_eq!(bins[9].count, 3);
    approx(bins[9].mean_predicted, (0.95 + 0.95 + 1.0) / 3.0);
    approx(bins[9].observed_rate, 1.0);
}

#[test]
fn band_classification_boundaries() {
    assert_eq!(popularity_band(Some(1)), "1番人気");
    assert_eq!(popularity_band(Some(3)), "2-3番人気");
    assert_eq!(popularity_band(Some(4)), "4-6番人気");
    assert_eq!(popularity_band(Some(9)), "7-9番人気");
    assert_eq!(popularity_band(Some(10)), "10番人気以下");
    assert_eq!(popularity_band(None), "人気不明");

    assert_eq!(field_size_band(9), "～9頭");
    assert_eq!(field_size_band(10), "10-12頭");
    assert_eq!(field_size_band(15), "13-15頭");
    assert_eq!(field_size_band(18), "16頭以上");
}

#[test]
fn band_functions_only_emit_declared_labels() {
    // band 関数の戻り値は必ず出力順定義の定数配列に含まれること。片方だけ変更したときの
    // 同期ずれ（セグメントが無言でドロップされる）をコンパイル時でなくテストで検出する。
    for pop in [
        None,
        Some(0u32),
        Some(1),
        Some(3),
        Some(6),
        Some(9),
        Some(18),
        Some(100),
    ] {
        assert!(
            POPULARITY_BANDS.contains(&popularity_band(pop)),
            "popularity_band({pop:?}) が POPULARITY_BANDS に無い"
        );
    }
    for n in [0usize, 8, 9, 10, 12, 15, 16, 30] {
        assert!(
            FIELD_SIZE_BANDS.contains(&field_size_band(n)),
            "field_size_band({n}) が FIELD_SIZE_BANDS に無い"
        );
    }
    for s in [Surface::Turf, Surface::Dirt] {
        assert!(
            SURFACE_BANDS.contains(&surface_band(s)),
            "surface_band({s:?}) が SURFACE_BANDS に無い"
        );
    }
}

#[test]
fn popularity_segments_group_entries_in_band_order() {
    let races = vec![RaceEvaluation {
        horses: vec![
            horse(0.5, 0.6, 0.7, Some(1), Some(1)), // 1番人気・勝ち
            horse(0.2, 0.3, 0.4, Some(3), Some(2)), // 2-3番人気・負け
            horse(0.1, 0.2, 0.3, Some(2), Some(3)), // 2-3番人気・負け
            horse(0.05, 0.1, 0.2, Some(5), None),   // 人気不明
        ],
        top_pick_position: Some(1),
        top_pick_odds: None,
        surface: Surface::Turf,
    }];
    let r = evaluate(&races);

    // 出力順は POPULARITY_BANDS 順、データのある帯のみ。
    let labels: Vec<&str> = r.by_popularity.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["1番人気", "2-3番人気", "人気不明"]);

    let fav = &r.by_popularity[0];
    assert_eq!(fav.entries, 1);
    approx(fav.mean_win_prob, 0.5);
    approx(fav.observed_win_rate, 1.0);
    // #258: place/show も帯別に出る。1番人気馬(place 0.6, show 0.7)は 1 着で連対・複勝とも的中。
    approx(fav.mean_place_prob, 0.6);
    approx(fav.observed_place_rate, 1.0);
    approx(fav.mean_show_prob, 0.7);
    approx(fav.observed_show_rate, 1.0);

    let band23 = &r.by_popularity[1];
    assert_eq!(band23.entries, 2);
    approx(band23.mean_win_prob, (0.2 + 0.1) / 2.0);
    approx(band23.observed_win_rate, 0.0);
    // 2 頭(place 0.3/0.2, show 0.4/0.3, 着順 3/2)。連対は pos2 のみ、複勝は両方。
    approx(band23.mean_place_prob, (0.3 + 0.2) / 2.0);
    approx(band23.observed_place_rate, 0.5);
    approx(band23.mean_show_prob, (0.4 + 0.3) / 2.0);
    approx(band23.observed_show_rate, 1.0);
}

#[test]
fn field_size_segments_group_races_in_band_order() {
    // 8 頭立て(～9頭)を 2 レース、14 頭立て(13-15頭)を 1 レース。
    let small = |pos: Option<u32>| RaceEvaluation {
        horses: (0..8).map(|_| win_horse(0.1, Some(2))).collect(),
        top_pick_position: pos,
        top_pick_odds: None,
        surface: Surface::Turf,
    };
    let large = RaceEvaluation {
        horses: (0..14).map(|_| win_horse(0.07, Some(2))).collect(),
        top_pick_position: Some(1),
        top_pick_odds: None,
        surface: Surface::Turf,
    };
    let races = vec![small(Some(1)), small(Some(5)), large];
    let r = evaluate(&races);

    let labels: Vec<&str> = r.by_field_size.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["～9頭", "13-15頭"]);

    let s = &r.by_field_size[0];
    assert_eq!(s.races, 2);
    approx(s.win_hit_rate, 0.5); // 1 着は 1 レースのみ

    let l = &r.by_field_size[1];
    assert_eq!(l.races, 1);
    approx(l.win_hit_rate, 1.0);
}

#[test]
fn surface_segments_group_races_in_band_order() {
    // 芝 2 レース（うち本命1着は1つ）、ダート 1 レース（本命1着）。
    let race = |surface: Surface, pos: Option<u32>| RaceEvaluation {
        horses: vec![win_horse(0.5, pos), win_horse(0.5, Some(9))],
        top_pick_position: pos,
        top_pick_odds: None,
        surface,
    };
    let races = vec![
        race(Surface::Turf, Some(1)),
        race(Surface::Turf, Some(4)),
        race(Surface::Dirt, Some(1)),
    ];
    let r = evaluate(&races);

    // 出力順は SURFACE_BANDS 順（芝→ダート）、データのある馬場のみ。
    let labels: Vec<&str> = r.by_surface.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["芝", "ダート"]);

    let turf = &r.by_surface[0];
    assert_eq!(turf.races, 2);
    approx(turf.win_hit_rate, 0.5); // 芝 2 戦で本命1着は1つ
    approx(turf.show_hit_rate, 0.5); // 本命の着順は 1 着と 4 着

    let dirt = &r.by_surface[1];
    assert_eq!(dirt.races, 1);
    approx(dirt.win_hit_rate, 1.0);

    // 片側馬場のみの入力では、その馬場 1 要素だけが返る（データのある馬場のみ）。
    let dirt_only = evaluate(&[race(Surface::Dirt, Some(1))]);
    let dirt_labels: Vec<&str> = dirt_only
        .by_surface
        .iter()
        .map(|s| s.label.as_str())
        .collect();
    assert_eq!(dirt_labels, vec!["ダート"]);
}

#[test]
fn exotic_segments_group_and_aggregate_by_type() {
    let bets = vec![
        ExoticBet {
            bet_type: "quinella",
            predicted_prob: 0.3,
            hit: true,
            odds: 5.0,
        },
        ExoticBet {
            bet_type: "quinella",
            predicted_prob: 0.2,
            hit: false,
            odds: 8.0,
        },
        ExoticBet {
            bet_type: "trifecta",
            predicted_prob: 0.05,
            hit: false,
            odds: 50.0,
        },
    ];
    let segs = exotic_segments(&bets);
    // EXOTIC_BET_TYPES 順（quinella→…→trifecta）、データのある券種のみ。
    let labels: Vec<&str> = segs.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["quinella", "trifecta"]);

    let q = &segs[0];
    assert_eq!(q.bets, 2);
    approx(q.mean_predicted, 0.25);
    approx(q.hit_rate, 0.5);
    approx(q.payout_rate, 5.0 / 2.0); // 的中 1 点(odds5.0) / 2 点

    let t = &segs[1];
    assert_eq!(t.bets, 1);
    approx(t.hit_rate, 0.0);
    approx(t.payout_rate, 0.0);
}

#[test]
fn exotic_payout_rate_sums_all_hits_over_total_bets() {
    // 同一券種で 2 点的中（賭け金一定前提）。回収率 = (的中オッズの和) / 総点数。
    let bets = vec![
        ExoticBet {
            bet_type: "win",
            predicted_prob: 0.5,
            hit: true,
            odds: 2.0,
        },
        ExoticBet {
            bet_type: "win",
            predicted_prob: 0.4,
            hit: true,
            odds: 3.0,
        },
        ExoticBet {
            bet_type: "win",
            predicted_prob: 0.3,
            hit: false,
            odds: 4.0,
        },
    ];
    let segs = exotic_segments(&bets);
    assert_eq!(segs.len(), 1);
    let w = &segs[0];
    assert_eq!(w.bets, 3);
    approx(w.hit_rate, 2.0 / 3.0);
    // (2.0 + 3.0) / 3 点 = 5/3。1 点でも複数的中でも分母は総点数。
    approx(w.payout_rate, 5.0 / 3.0);
}

#[test]
fn top3_rank_distribution_buckets_by_model_show_rank() {
    // 8 頭、show_prob 降順（h0=0.80 … h7=0.10）＝モデル順位 = index+1。
    // 3 着内入線: h0(rank1, 3着)→1-3 / h4(rank5, 2着)→4-6 / h7(rank8, 1着)→7+。
    let s = |idx: usize| 0.80 - 0.10 * idx as f64;
    let mk = |idx: usize, pos: Option<u32>| horse(s(idx), s(idx), s(idx), pos, None);
    let races = vec![RaceEvaluation {
        horses: vec![
            mk(0, Some(3)), // rank1 → 1-3
            mk(1, Some(4)),
            mk(2, Some(5)),
            mk(3, Some(6)),
            mk(4, Some(2)), // rank5 → 4-6
            mk(5, Some(7)),
            mk(6, None),
            mk(7, Some(1)), // rank8 → 7+
        ],
        top_pick_position: Some(3),
        top_pick_odds: None,
        surface: Surface::Turf,
    }];
    let d = evaluate(&races).top3_rank_distribution;
    assert_eq!(d.finishers, 3);
    assert_eq!(d.model_rank_1_3, 1);
    assert_eq!(d.model_rank_4_6, 1);
    assert_eq!(d.model_rank_7_plus, 1);
}

#[test]
fn place_and_show_reliability_are_populated() {
    // 評価レースがあれば place/show の reliability 曲線も win と同じ 10 ビンで埋まる（#258）。
    let races = vec![RaceEvaluation {
        horses: vec![
            horse(0.5, 0.7, 0.9, Some(1), Some(1)),
            horse(0.3, 0.5, 0.7, Some(4), Some(2)),
        ],
        top_pick_position: Some(1),
        top_pick_odds: None,
        surface: Surface::Turf,
    }];
    let r = evaluate(&races);
    assert_eq!(r.place_reliability.len(), 10);
    assert_eq!(r.show_reliability.len(), 10);
    // show_prob 0.9 の馬は最終ビン[0.9,1.0]に入り 3着以内(1着)で実率 1.0。
    let top_bin = r.show_reliability.last().unwrap();
    assert_eq!(top_bin.count, 1);
    approx(top_bin.observed_rate, 1.0);
}
