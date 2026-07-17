//! 集計・バックテスト結果を整形して標準出力に書く表示専用モジュール（#417）。
//! 各関数は「整形して println! する」責務のみを持ち、計算やデータ取得は行わない。

use chrono::NaiveDate;
use paddock_domain::{
    BacktestReport, ExoticSegment, FieldSizeSegment, HorseNum, HorseProbability, PairEvDiagnostic,
    PopularitySegment, ReliabilityBin, SurfaceSegment, Top3RankDistribution,
};
use paddock_use_case::repository::{
    CourseStatsRow, GroupStat, HorseStatsRow, JockeyStatsRow, TrainerStatsRow,
};

/// 候補が複数ある場合に一覧を提示して終了する（ユーザーが絞り込んで再実行）。
/// `truncated` が true のとき、表示件数を超える候補がある旨を示す。
pub(crate) fn print_candidates(kind: &str, query: &str, names: &[String], truncated: bool) {
    let count = if truncated {
        format!("{} 件以上", names.len())
    } else {
        format!("{} 件", names.len())
    };
    println!("「{query}」に一致する{kind}が {count} あります。絞り込んで再実行してください:");
    for n in names {
        println!("  - {n}");
    }
}

pub(crate) fn print_horse(s: &HorseStatsRow) {
    println!("# 馬 {}", s.horse_name);
    println!();
    print_section("全体", std::slice::from_ref(&s.overall));
    print_section("芝/ダート", &s.by_surface);
    print_section("距離帯", &s.by_distance_band);
    print_section("枠順", &s.by_gate_group);
    print_section("馬場状態", &s.by_track_condition);
    print_section("人気帯", &s.by_popularity_band);
}

pub(crate) fn print_course(s: &CourseStatsRow) {
    println!("# コース {} {}m {}", s.venue, s.distance, s.surface);
    println!();
    print_section("枠順", &s.by_gate_group);
}

pub(crate) fn print_jockey(s: &JockeyStatsRow) {
    println!("# 騎手 {}", s.jockey_name);
    println!();
    print_section("全体", std::slice::from_ref(&s.overall));
    print_section("芝/ダート", &s.by_surface);
    print_section("枠順", &s.by_gate_group);
}

pub(crate) fn print_trainer(s: &TrainerStatsRow) {
    println!("# 調教師 {}", s.trainer_name);
    println!();
    print_section("全体", std::slice::from_ref(&s.overall));
    print_section("芝/ダート", &s.by_surface);
    print_section("枠順", &s.by_gate_group);
}

/// 軸-相手ペアの「馬連 vs 馬単(両方向)」EV を並べる診断表（#246-C）。
/// 着順不問の馬連と、本命→相手 / 相手→本命 の馬単 EV を比較して券種選択の判断材料にする。
/// EV は的中確率 × オッズ。オッズ未取得のセルは `—`。軸は `pair_ev_diagnostics` が決めた
/// canonical な値（use-case 経由）を受け取り、ここで再計算しない。
pub(crate) fn print_pair_ev_diagnostics(
    axis: Option<HorseNum>,
    probs: &[HorseProbability],
    rows: &[PairEvDiagnostic],
) {
    if rows.is_empty() {
        return;
    }
    let name_of = |num| {
        probs
            .iter()
            .find(|p| p.horse_num == num)
            .map(|p| p.horse_name.value().to_string())
            .unwrap_or_default()
    };
    match axis {
        Some(a) => println!(
            "\n# 馬連 vs 馬単 EV 診断（軸 {} {}）",
            a.value(),
            name_of(a)
        ),
        None => return,
    }
    println!(
        "{:<18} {:>16} {:>16} {:>16}",
        "相手", "馬連EV(オッズ)", "馬単 軸→相手", "馬単 相手→軸"
    );
    for r in rows {
        let label = format!("{} {}", r.partner.value(), name_of(r.partner));
        println!(
            "{:<18} {:>16} {:>16} {:>16}",
            label,
            fmt_ev_odds(r.quinella_ev, r.quinella_odds),
            fmt_ev_odds(r.exacta_fwd_ev, r.exacta_fwd_odds),
            fmt_ev_odds(r.exacta_rev_ev, r.exacta_rev_odds),
        );
    }
}

/// EV とオッズを `EV(オッズ)` で整形。オッズ未取得は `—`。
fn fmt_ev_odds(ev: f64, odds: Option<f64>) -> String {
    match odds {
        Some(o) => format!("{ev:.2}({o:.1})"),
        None => "—".to_string(),
    }
}

pub(crate) fn print_backtest(from: NaiveDate, to: NaiveDate, r: &BacktestReport) {
    println!("# バックテスト {from} 〜 {to}");
    if r.races_evaluated == 0 {
        println!("評価対象レースなし");
        return;
    }
    println!("{:<20}: {}", "評価レース数", r.races_evaluated);
    println!("{:<20}: {:>5.1}%", "単勝的中率", r.win_hit_rate * 100.0);
    println!("{:<20}: {:>5.1}%", "連対的中率", r.place_hit_rate * 100.0);
    println!("{:<20}: {:>5.1}%", "複勝的中率", r.show_hit_rate * 100.0);
    match r.payout_rate {
        Some(rate) => println!(
            "{:<20}: {:>5.1}%  (母数 {} レース)",
            "想定回収率",
            rate * 100.0,
            r.payout_races
        ),
        None => println!("{:<20}: —  (母数 0 レース)", "想定回収率"),
    }
    println!();

    // 確率校正（Brier / LogLoss）。単勝・連対・複勝の各確率について算出。
    println!("## 確率校正（小さいほど良い）");
    println!("{:<8} {:>10} {:>10}", "種別", "Brier", "LogLoss");
    println!("{:<8} {:>10.4} {:>10.4}", "単勝", r.brier, r.log_loss);
    println!(
        "{:<8} {:>10.4} {:>10.4}",
        "連対", r.place_calibration.brier, r.place_calibration.log_loss
    );
    println!(
        "{:<8} {:>10.4} {:>10.4}",
        "複勝", r.show_calibration.brier, r.show_calibration.log_loss
    );
    println!();

    print_reliability(&r.win_reliability, "単勝");
    print_reliability(&r.place_reliability, "連対");
    print_reliability(&r.show_reliability, "複勝");
    print_top3_rank_distribution(&r.top3_rank_distribution);
    print_surface_segments(&r.by_surface);
    print_field_size_segments(&r.by_field_size);
    print_popularity_segments(&r.by_popularity);
    print_placeshow_popularity(&r.by_popularity);
    print_exotic_segments(&r.by_exotic);

    println!(
        "※ 想定回収率は「トップ選好馬の単勝に毎レース 100 円」固定の参考値（本番の EV/Kelly 買い目とは別戦略）。"
    );
}

/// `kind`（単勝/連対/複勝）の reliability 曲線（予測確率帯ごとの平均予測 vs 実測率）。空ビンは省略。
fn print_reliability(bins: &[ReliabilityBin], kind: &str) {
    // 全ビン空（= 評価レース 0 件）なら見出しごと省略する。3 券種を出すようになり（#258）、
    // 空表 3 つを並べないため。評価レースがあれば必ず非空ビンができるので通常は出力される。
    if bins.iter().all(|b| b.count == 0) {
        return;
    }
    println!("## reliability 曲線（{kind}・予測確率帯ごと）");
    println!(
        "{:<10} {:>6} {:>10} {:>10}",
        "確率帯", "件数", "平均予測", "実測率"
    );
    for b in bins {
        if b.count == 0 {
            continue;
        }
        println!(
            "{:<10} {:>6} {:>9.1}% {:>9.1}%",
            format!("{:.0}–{:.0}%", b.lower * 100.0, b.upper * 100.0),
            b.count,
            b.mean_predicted * 100.0,
            b.observed_rate * 100.0,
        );
    }
    println!();
    println!("（平均予測 ≒ 実測率なら校正良好。平均予測 > 実測なら過大評価、< なら過小評価。）");
    println!();
}

/// 3 着以内入線馬のモデル複勝(show_prob)順位分布（#258）。7 位以下が多いほど、複勝圏に来る
/// 人気薄をモデルが下位に沈めて取りこぼしている（複勝圏の過小評価の直接指標）。
fn print_top3_rank_distribution(d: &Top3RankDistribution) {
    if d.finishers == 0 {
        return;
    }
    let pct = |x: u32| x as f64 / d.finishers as f64 * 100.0;
    println!("## 3着内入線馬のモデル複勝順位分布（#258）");
    println!(
        "入線 {} 頭 — モデル順位 1-3位: {} ({:.1}%) / 4-6位: {} ({:.1}%) / 7位以下: {} ({:.1}%)",
        d.finishers,
        d.model_rank_1_3,
        pct(d.model_rank_1_3),
        d.model_rank_4_6,
        pct(d.model_rank_4_6),
        d.model_rank_7_plus,
        pct(d.model_rank_7_plus),
    );
    println!("（7位以下が多いほど、複勝圏に来る人気薄をモデルが下位に沈めて取りこぼしている。）");
    println!();
}

/// 頭数帯別の集計（的中率＋単勝校正）。
fn print_field_size_segments(segs: &[FieldSizeSegment]) {
    if segs.is_empty() {
        return;
    }
    println!("## 頭数帯別");
    println!(
        "{:<10} {:>5} {:>8} {:>8} {:>8} {:>9} {:>9}",
        "頭数帯", "R数", "単勝率", "連対率", "複勝率", "Brier", "LogLoss"
    );
    for s in segs {
        println!(
            "{:<10} {:>5} {:>7.1}% {:>7.1}% {:>7.1}% {:>9.4} {:>9.4}",
            s.label,
            s.races,
            s.win_hit_rate * 100.0,
            s.place_hit_rate * 100.0,
            s.show_hit_rate * 100.0,
            s.win_calibration.brier,
            s.win_calibration.log_loss,
        );
    }
    println!();
}

/// 馬場（芝/ダート）別の集計（的中率＋単勝校正）。馬場別 α の要否検討用（#113）。
fn print_surface_segments(segs: &[SurfaceSegment]) {
    if segs.is_empty() {
        return;
    }
    println!("## 芝/ダート別");
    println!(
        "{:<8} {:>5} {:>8} {:>8} {:>8} {:>9} {:>9}",
        "馬場", "R数", "単勝率", "連対率", "複勝率", "Brier", "LogLoss"
    );
    for s in segs {
        println!(
            "{:<8} {:>5} {:>7.1}% {:>7.1}% {:>7.1}% {:>9.4} {:>9.4}",
            s.label,
            s.races,
            s.win_hit_rate * 100.0,
            s.place_hit_rate * 100.0,
            s.show_hit_rate * 100.0,
            s.win_calibration.brier,
            s.win_calibration.log_loss,
        );
    }
    println!();
}

/// 人気帯別の単勝校正（平均予測 vs 実測勝率）。
fn print_popularity_segments(segs: &[PopularitySegment]) {
    if segs.is_empty() {
        return;
    }
    println!("## 人気帯別（単勝校正）");
    println!(
        "{:<12} {:>6} {:>10} {:>10} {:>9} {:>9}",
        "人気帯", "頭数", "平均予測", "実測勝率", "Brier", "LogLoss"
    );
    for s in segs {
        println!(
            "{:<12} {:>6} {:>9.1}% {:>9.1}% {:>9.4} {:>9.4}",
            s.label,
            s.entries,
            s.mean_win_prob * 100.0,
            s.observed_win_rate * 100.0,
            s.win_calibration.brier,
            s.win_calibration.log_loss,
        );
    }
    println!();
}

/// 人気帯別の複勝圏（place/show）過小評価診断（#258）。差 = 実率 − 平均予測。
/// 人気薄帯で複勝差が大きく正なら、モデルは複勝圏に来る人気薄を過小評価している。
fn print_placeshow_popularity(segs: &[PopularitySegment]) {
    if segs.is_empty() {
        return;
    }
    println!("## 人気帯別 複勝圏 過小評価診断（#258）");
    println!(
        "{:<12} {:>6} {:>9} {:>9} {:>8} {:>9} {:>9} {:>8}",
        "人気帯", "頭数", "予測連対", "実連対", "連対差", "予測複勝", "実複勝", "複勝差"
    );
    for s in segs {
        let place_gap = s.observed_place_rate - s.mean_place_prob;
        let show_gap = s.observed_show_rate - s.mean_show_prob;
        println!(
            "{:<12} {:>6} {:>8.1}% {:>8.1}% {:>+7.1}% {:>8.1}% {:>8.1}% {:>+7.1}%",
            s.label,
            s.entries,
            s.mean_place_prob * 100.0,
            s.observed_place_rate * 100.0,
            place_gap * 100.0,
            s.mean_show_prob * 100.0,
            s.observed_show_rate * 100.0,
            show_gap * 100.0,
        );
    }
    println!();
    println!("（差 = 実率 − 平均予測。人気薄帯で複勝差が大きく正なら複勝圏の過小評価が確定。）");
    println!();
}

/// 買い目（curated 推奨）の券種別 校正・回収率（#121）。空（当時オッズが無い等）なら省略。
/// 過信なら「平均予測 ≫ 実的中率」。回収率は 1 点 100 円固定の的中オッズ平均。
fn print_exotic_segments(segs: &[ExoticSegment]) {
    if segs.is_empty() {
        return;
    }
    println!("## 買い目（curated）券種別 校正・回収率");
    println!(
        "{:<10} {:>6} {:>10} {:>10} {:>9} {:>9} {:>9}",
        "券種", "点数", "平均予測", "実的中率", "Brier", "LogLoss", "回収率"
    );
    for s in segs {
        println!(
            "{:<10} {:>6} {:>9.1}% {:>9.1}% {:>9.4} {:>9.4} {:>8.1}%",
            s.label,
            s.bets,
            s.mean_predicted * 100.0,
            s.hit_rate * 100.0,
            s.calibration.brier,
            s.calibration.log_loss,
            s.payout_rate * 100.0,
        );
    }
    println!(
        "※ 回収率は「1 点 100 円固定・複勝は中央値近似」の参考値。実払戻の端数処理や軸流し/予算配分（#122）は含まない。"
    );
    println!(
        "※ 小頭数(7頭以下)の複勝/ワイドは採用確率(3着以内)と的中定義(2着以内)が非対称で平均予測が実的中率を上回りやすく、同着レースは一部券種で取りこぼす（いずれも計測アーティファクト）。"
    );
    println!();
}

fn print_section(title: &str, rows: &[GroupStat]) {
    println!("## {title}");
    println!(
        "{:<14} {:>6} {:>6} {:>6} {:>9} {:>9}",
        "区分", "出走", "勝", "連対", "勝率", "連対率"
    );
    for r in rows {
        println!(
            "{:<14} {:>6} {:>6} {:>6} {:>8.1}% {:>8.1}%",
            r.label,
            r.starts,
            r.wins,
            r.places,
            r.win_rate() * 100.0,
            r.place_rate() * 100.0
        );
    }
    println!();
}
