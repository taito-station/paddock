mod cli;
mod printer;
mod setup;

use std::collections::HashMap;

use chrono::{Months, NaiveDate, Utc};
use clap::Parser;
use paddock_domain::{
    EstimationConfig, FactorStat, FeatureRow, HorseName, HorseNum, JockeyName, PortfolioConfig,
    RECOMMENDED_MARKET_BLEND_ALPHA, RaceId, RecencyConfig, ShrinkageConfig, Surface, TrainerName,
    Venue, pair_ev_diagnostics,
};
use paddock_use_case::{PredictionViews, TREND_N_MAX};
use predict_format::{format_probs, format_probs_with_market};

/// 部分一致候補の表示上限。これを超える場合も先頭から打ち切って提示する。
const CANDIDATE_LIMIT: u32 = 20;

/// 特徴量ダンプ（#272 Phase A / #309）TSV の列数。[`FEATURE_DUMP_HEADER`] と [`feature_row_cells`] の
/// 双方をこの不変条件で縛り、列ズレ（＝学習データの静かな汚染）を防ぐ（ユニットテストで担保）。
/// 内訳: id3(race_id/date/horse_num) + 10 factor × (win,place,show,starts)=40 + signal4 + model3 + ラベル3。
/// signal4 = recent_form/weight_carried/jockey_recent_form/running_style（#329 Phase1 で running_style 追加）。
/// 10 factor = 既存 6 + #350 相性 4（jockey_venue/jockey_distance/jockey_horse_combo/horse_venue）。
const FEATURE_DUMP_COLUMNS: usize = 53;

/// 特徴量ダンプ（#272 Phase A / #309）TSV のヘッダ行。列順は [`feature_row_cells`] の行生成と一致させ、
/// 列数は [`FEATURE_DUMP_COLUMNS`] と一致させる（いずれもユニットテストで担保）。`model_*` は内蔵モデルの
/// 最終確率（backtest が評価するのと同一値）で、Python ハーネス③の忠実性サニティの基準に使う。
const FEATURE_DUMP_HEADER: &str = "race_id\tdate\thorse_num\t\
course_gate_win\tcourse_gate_place\tcourse_gate_show\tcourse_gate_starts\t\
horse_surface_win\thorse_surface_place\thorse_surface_show\thorse_surface_starts\t\
horse_distance_win\thorse_distance_place\thorse_distance_show\thorse_distance_starts\t\
jockey_surface_win\tjockey_surface_place\tjockey_surface_show\tjockey_surface_starts\t\
trainer_surface_win\ttrainer_surface_place\ttrainer_surface_show\ttrainer_surface_starts\t\
horse_track_condition_win\thorse_track_condition_place\thorse_track_condition_show\thorse_track_condition_starts\t\
jockey_venue_win\tjockey_venue_place\tjockey_venue_show\tjockey_venue_starts\t\
jockey_distance_win\tjockey_distance_place\tjockey_distance_show\tjockey_distance_starts\t\
jockey_horse_combo_win\tjockey_horse_combo_place\tjockey_horse_combo_show\tjockey_horse_combo_starts\t\
horse_venue_win\thorse_venue_place\thorse_venue_show\thorse_venue_starts\t\
recent_form\tweight_carried\tjockey_recent_form\trunning_style\t\
model_win\tmodel_place\tmodel_show\t\
finishing_position\twin_odds\tpopularity";

/// 1 行分の特徴量を [`FEATURE_DUMP_HEADER`] と同じ列順の文字列セル列に展開する。欠落（`None`）は
/// 空セルで 0 埋めしない（欠落項とレート 0 を区別する）。数値は `f64`/`u32` の既定 Display
/// （round-trip 可能な厳密値）で出力し、忠実性サニティで backtest 集計と突合できるようにする。
fn feature_row_cells(row: &FeatureRow) -> Vec<String> {
    // factor 1 つを (win,place,show,starts) の 4 セルに展開する。欠落項は 4 セルとも空。
    fn push_stat(cells: &mut Vec<String>, stat: Option<FactorStat>) {
        match stat {
            Some(s) => {
                cells.push(s.rate.win.to_string());
                cells.push(s.rate.place.to_string());
                cells.push(s.rate.show.to_string());
                cells.push(s.starts.to_string());
            }
            None => {
                for _ in 0..4 {
                    cells.push(String::new());
                }
            }
        }
    }
    let cell_f64 = |v: Option<f64>| v.map(|x| x.to_string()).unwrap_or_default();
    let cell_u32 = |v: Option<u32>| v.map(|x| x.to_string()).unwrap_or_default();

    let mut cells: Vec<String> = Vec::with_capacity(FEATURE_DUMP_COLUMNS);
    cells.push(row.race_id.clone());
    cells.push(row.date.to_string());
    cells.push(row.horse_num.to_string());
    push_stat(&mut cells, row.factors.course_gate);
    push_stat(&mut cells, row.factors.horse_surface);
    push_stat(&mut cells, row.factors.horse_distance);
    push_stat(&mut cells, row.factors.jockey_surface);
    push_stat(&mut cells, row.factors.trainer_surface);
    push_stat(&mut cells, row.factors.horse_track_condition);
    // #350 相性 factor（既存 6 factor の直後・signal4 の直前に 4 factor × 4 セル）。
    push_stat(&mut cells, row.factors.jockey_venue);
    push_stat(&mut cells, row.factors.jockey_distance);
    push_stat(&mut cells, row.factors.jockey_horse_combo);
    push_stat(&mut cells, row.factors.horse_venue);
    cells.push(cell_f64(row.factors.recent_form));
    cells.push(cell_f64(row.factors.weight_carried));
    cells.push(cell_f64(row.factors.jockey_recent_form));
    cells.push(cell_f64(row.factors.running_style));
    // 内蔵モデルの最終確率（必ず付く・欠落なし）。Python ハーネス③が backtest 数値との一致に使う。
    cells.push(row.model_win.to_string());
    cells.push(row.model_place.to_string());
    cells.push(row.model_show.to_string());
    cells.push(cell_u32(row.finishing_position));
    cells.push(cell_f64(row.win_odds));
    cells.push(cell_u32(row.popularity));
    // ヘッダと行の列数ズレを開発時に即検知する（出力契約の保険。本数値はテストでも担保）。
    debug_assert_eq!(
        cells.len(),
        FEATURE_DUMP_COLUMNS,
        "feature dump の列数がヘッダと不一致"
    );
    // TSV のセルに区切り文字が混入すると静かに列ズレする。現状の列（英数字+`-` の race_id・
    // NaiveDate・数値）はタブ/改行を含まないが、ソース書式変更時の退行を開発時に検知する。
    debug_assert!(
        cells.iter().all(|c| !c.contains(['\t', '\n'])),
        "feature dump のセルに区切り文字(タブ/改行)が混入"
    );
    cells
}

/// 特徴量ダンプ（#272 Phase A）を TSV で書き出す。ヘッダ＋各行を [`feature_row_cells`] で生成する。
fn write_feature_dump(path: &str, rows: &[FeatureRow]) -> anyhow::Result<()> {
    use std::io::Write;
    let file = std::fs::File::create(path)?;
    let mut w = std::io::BufWriter::new(file);
    writeln!(w, "{FEATURE_DUMP_HEADER}")?;
    for row in rows {
        writeln!(w, "{}", feature_row_cells(row).join("\t"))?;
    }
    w.flush()?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    let app = setup::build_app().await?;

    match args.command {
        cli::Command::Horse { name } => {
            // 入力を正規化（半角カナ→全角等）してから results を中間一致で検索する。
            let query = HorseName::try_from(name.as_str())?;
            // 打ち切りを検出するため上限 +1 件取得する。
            let mut candidates = app
                .interactor
                .find_horse_candidates(query.value(), CANDIDATE_LIMIT + 1)
                .await?;
            let truncated = candidates.len() as u32 > CANDIDATE_LIMIT;
            candidates.truncate(CANDIDATE_LIMIT as usize);
            match candidates.as_slice() {
                [] => println!("該当する馬が見つかりません: {name}"),
                [one] => {
                    let h = HorseName::try_from(one.as_str())?;
                    let stats = app.interactor.horse_stats(&h).await?;
                    printer::print_horse(&stats);
                }
                many => printer::print_candidates("馬", &name, many, truncated),
            }
        }
        cli::Command::Course {
            venue,
            distance,
            surface,
        } => {
            let v = Venue::try_from(venue.as_str())?;
            let s = Surface::try_from(surface.as_str())?;
            let stats = app.interactor.course_stats(v, distance, s).await?;
            printer::print_course(&stats);
        }
        cli::Command::Jockey { name } => {
            let query = JockeyName::try_from(name.as_str())?;
            let mut candidates = app
                .interactor
                .find_jockey_candidates(query.value(), CANDIDATE_LIMIT + 1)
                .await?;
            let truncated = candidates.len() as u32 > CANDIDATE_LIMIT;
            candidates.truncate(CANDIDATE_LIMIT as usize);
            match candidates.as_slice() {
                [] => println!("該当する騎手が見つかりません: {name}"),
                [one] => {
                    let j = JockeyName::try_from(one.as_str())?;
                    let stats = app.interactor.jockey_stats(&j).await?;
                    printer::print_jockey(&stats);
                }
                many => printer::print_candidates("騎手", &name, many, truncated),
            }
        }
        cli::Command::Trainer { name } => {
            let query = TrainerName::try_from(name.as_str())?;
            let mut candidates = app
                .interactor
                .find_trainer_candidates(query.value(), CANDIDATE_LIMIT + 1)
                .await?;
            let truncated = candidates.len() as u32 > CANDIDATE_LIMIT;
            candidates.truncate(CANDIDATE_LIMIT as usize);
            match candidates.as_slice() {
                [] => println!("該当する調教師が見つかりません: {name}"),
                [one] => {
                    let t = TrainerName::try_from(one.as_str())?;
                    let stats = app.interactor.trainer_stats(&t).await?;
                    printer::print_trainer(&stats);
                }
                many => printer::print_candidates("調教師", &name, many, truncated),
            }
        }
        cli::Command::Predict {
            race_id,
            blend_alpha,
            track_condition,
            shrinkage_m,
            win_power,
        } => {
            // 未指定時は本番既定 α=0.2（session.rs / predict-watch と対称）。過去データ視点は常に
            // 純モデルなので、この α は市場EV視点の軸/相手ランキングにのみ効く。
            let blend_alpha = validate_blend_alpha(blend_alpha)?.or(RECOMMENDED_MARKET_BLEND_ALPHA);
            // #282: 本番既定（m=10/γ=1.25）を土台に、指定フラグだけ上書きした config を組む。
            // #270/ADR 0045 の m×α×γ 再検証で m/γ を振った bt_pred を生成するために使う。
            let config = production_config_with_overrides(shrinkage_m, win_power)?;
            let rid = RaceId::try_from(race_id.as_str())?;
            // #272 ③④: session.rs と同じ二視点で出す。過去データ視点＝純モデル（α=1.0・市場非依存）、
            // 市場EV視点＝軸/相手 blended・EV pure（循環断ち）。--blend-alpha は市場EV視点の順位付け用。
            let (views, odds) = app
                .interactor
                .predict_race_views_with_odds(&rid, blend_alpha, track_condition, false, &config)
                .await?;
            let PredictionViews {
                blended,
                pure,
                explanations: _,
            } = views;

            // 過去データ視点（純モデル）: 市場に依らない公開データだけの読み。
            println!("【過去データ視点（純モデル）】");
            for line in format_probs(&pure) {
                println!("{line}");
            }

            match odds {
                Some(odds) => {
                    // 純モデル勝率 vs 市場implied（控除率除去）。差＝割安/割高を読む材料。
                    let market_win: HashMap<HorseNum, f64> =
                        odds.win.iter().map(|(num, o)| (*num, o.value())).collect();
                    println!();
                    println!("【純モデル vs 市場implied】");
                    // analyze predict は根拠（explanations）を組まないため枠妙味フラグは空（#343 は
                    // 通常 predict フロー=predict/predict-watch で提示）。市場差分列は従来どおり出す。
                    for line in format_probs_with_market(&pure, &market_win, &HashMap::new()) {
                        println!("{line}");
                    }

                    // 市場EV視点: 軸/相手=blended・EV/的中=pure（#272 循環断ち）のペアEV診断。
                    // 見出しは print_pair_ev_diagnostics 側（"# 馬連 vs 馬単 EV 診断（軸 …）"）に集約する。
                    println!();
                    println!("【市場EV視点（軸/相手=市場ブレンド・EV=純モデル×odds）】");
                    let diag = pair_ev_diagnostics(
                        &blended,
                        &pure,
                        &odds,
                        PortfolioConfig::default().partners,
                    );
                    printer::print_pair_ev_diagnostics(diag.axis, &blended, &diag.rows);
                }
                None => {
                    println!();
                    println!(
                        "オッズ未取得 — 市場implied比較・EV視点は表示できません（過去データ視点のみ）"
                    );
                }
            }
        }
        cli::Command::Backtest {
            from,
            to,
            blend_alpha,
            shrinkage_m,
            recency_half_life,
            recent_form_weight,
            trend_n,
            jockey_form_weight,
            jockey_venue_weight,
            jockey_distance_weight,
            jockey_horse_combo_weight,
            horse_venue_weight,
            win_power,
            place_show_power,
            impute_missing_factors,
            dump_features,
        } => {
            let blend_alpha = validate_blend_alpha(blend_alpha)?;
            let config = build_estimation_config(
                shrinkage_m,
                recency_half_life,
                recent_form_weight,
                trend_n,
                jockey_form_weight,
                AffinityWeights {
                    jockey_venue: jockey_venue_weight,
                    jockey_distance: jockey_distance_weight,
                    jockey_horse_combo: jockey_horse_combo_weight,
                    horse_venue: horse_venue_weight,
                },
                win_power,
                place_show_power,
                impute_missing_factors,
            )?;
            let from = parse_date(&from)?;
            let to = parse_date(&to)?;
            let report = app
                .interactor
                .backtest(from, to, blend_alpha, config, dump_features.is_some())
                .await?;
            printer::print_backtest(from, to, &report);
            // --dump-features 指定時は特徴量ダンプを TSV に書く（#272 Phase A）。clean-arch のため
            // interactor は file IO せず report.feature_dump に行を載せて返し、ここで書き出す。
            if let Some(path) = dump_features {
                // dump_features.is_some() を渡しているので feature_dump は必ず Some。
                let rows = report
                    .feature_dump
                    .as_deref()
                    .expect("dump_features 要求時は feature_dump が埋まる");
                write_feature_dump(&path, rows)?;
                println!("特徴量ダンプ: {} 行を {path} に書き出し", rows.len());
            }
        }
        cli::Command::PurgeSnapshots { months, dry_run } => {
            // 0 ヶ月は当日以降のみ保持＝ほぼ全削除で #218 の蓄積を壊すため弾く。
            if months == 0 {
                anyhow::bail!("--months must be >= 1 (got {months})");
            }
            // fetched_at は UTC 基準なので cutoff も UTC の今日から引く。
            let cutoff = Utc::now()
                .date_naive()
                .checked_sub_months(Months::new(months))
                .ok_or_else(|| anyhow::anyhow!("cutoff date underflow for --months {months}"))?;
            let n = app
                .interactor
                .purge_old_race_odds_snapshots(cutoff, dry_run)
                .await?;
            if dry_run {
                println!(
                    "[dry-run] race_odds_snapshots: cutoff={cutoff} より前の {n} 行が削除対象（保持 {months} ヶ月）"
                );
            } else {
                println!(
                    "race_odds_snapshots: cutoff={cutoff} より前の {n} 行を削除（保持 {months} ヶ月）"
                );
            }
        }
    }

    Ok(())
}

fn parse_date(s: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("invalid date '{s}' (expected YYYY-MM-DD): {e}"))
}

/// `--blend-alpha` を検証する。未指定はそのまま `None`、指定時は `[0,1]` のみ許可。
fn validate_blend_alpha(alpha: Option<f64>) -> anyhow::Result<Option<f64>> {
    if let Some(a) = alpha
        && !((0.0..=1.0).contains(&a))
    {
        anyhow::bail!("--blend-alpha must be within [0, 1], got {a}");
    }
    Ok(alpha)
}

/// 有限の正数のみ許可するフラグ（縮約 m・冪 γ 系）を検証する。未指定は `None` を透過。
/// 0/負/非有限はゼロ除算・無意味なため usage エラーにする。`flag` はエラーメッセージ用のフラグ名。
fn validate_positive_finite(flag: &str, value: Option<f64>) -> anyhow::Result<Option<f64>> {
    if let Some(v) = value
        && !(v.is_finite() && v > 0.0)
    {
        anyhow::bail!("{flag} must be a finite positive number, got {v}");
    }
    Ok(value)
}

/// predict 用（#282）: 本番既定 `EstimationConfig::production()`（m=10/α=0.2/γ=1.25）を土台に、
/// 指定された `--shrinkage-m` / `--win-power` だけを上書きした config を組む。未指定フィールドは
/// 本番既定のまま（backtest の `build_estimation_config` が空 config から積むのと対照的に、predict は
/// 本番挙動の上に差分だけ載せる）。#270/ADR 0045 の m×α×γ 再検証 bt_pred 生成に使う。
fn production_config_with_overrides(
    shrinkage_m: Option<f64>,
    win_power: Option<f64>,
) -> anyhow::Result<EstimationConfig> {
    let mut config = EstimationConfig::production();
    if let Some(m) = validate_positive_finite("--shrinkage-m", shrinkage_m)? {
        config.shrinkage = Some(ShrinkageConfig { pseudo_count: m });
    }
    if let Some(g) = validate_positive_finite("--win-power", win_power)? {
        config.win_power = Some(g);
    }
    Ok(config)
}

/// backtest 用の確率推定設定（#75, #217, #220）を CLI フラグから組み立てる。未指定フラグは現行挙動。
/// `--shrinkage-m` / `--recency-half-life` とも指定時は有限の正数のみ許可
/// （0 や負値はゼロ除算・無意味なため）。`--recent-form-weight` は有限の非負数のみ、
/// `--trend-n` は 1〜3 のみ許可。
// 引数は backtest CLI の各スイープフラグと 1:1 対応（#75/#217/#220/#246/#283/#272）。まとめ struct 化は
// clap 側と本関数で二重定義になり見通しを損なうため、フラグ列をそのまま受ける。
/// #350 相性 factor の重み override（measure-first sweep）。`build_estimation_config` の引数肥大を
/// 抑えるためまとめて渡す。未指定は各 factor の const（0.0）＝寄与ゼロ。
#[derive(Debug, Clone, Copy, Default)]
struct AffinityWeights {
    jockey_venue: Option<f64>,
    jockey_distance: Option<f64>,
    jockey_horse_combo: Option<f64>,
    horse_venue: Option<f64>,
}

#[allow(clippy::too_many_arguments)]
fn build_estimation_config(
    shrinkage_m: Option<f64>,
    recency_half_life: Option<f64>,
    recent_form_weight: Option<f64>,
    trend_n: u32,
    jockey_form_weight: Option<f64>,
    affinity: AffinityWeights,
    win_power: Option<f64>,
    place_show_power: Option<f64>,
    impute_missing_factors: bool,
) -> anyhow::Result<EstimationConfig> {
    // 縮約 m・γ 系（有限正数のみ）は predict の override と同じ検証を共用する（#282）。
    let shrinkage = validate_positive_finite("--shrinkage-m", shrinkage_m)?
        .map(|m| ShrinkageConfig { pseudo_count: m });
    let recency = validate_positive_finite("--recency-half-life", recency_half_life)?
        .map(|h| RecencyConfig { half_life_days: h });
    let win_power = validate_positive_finite("--win-power", win_power)?;
    let place_show_power = validate_positive_finite("--place-show-power", place_show_power)?;
    if let Some(w) = recent_form_weight
        && !(w.is_finite() && w >= 0.0)
    {
        anyhow::bail!("--recent-form-weight must be a finite non-negative number, got {w}");
    }
    if let Some(w) = jockey_form_weight
        && !(w.is_finite() && w >= 0.0)
    {
        anyhow::bail!("--jockey-form-weight must be a finite non-negative number, got {w}");
    }
    // #350 相性 factor の weight override（有限非負のみ。recent_form_weight と同じ検証）。
    for (flag, w) in [
        ("--jockey-venue-weight", affinity.jockey_venue),
        ("--jockey-distance-weight", affinity.jockey_distance),
        ("--jockey-horse-combo-weight", affinity.jockey_horse_combo),
        ("--horse-venue-weight", affinity.horse_venue),
    ] {
        if let Some(w) = w
            && !(w.is_finite() && w >= 0.0)
        {
            anyhow::bail!("{flag} must be a finite non-negative number, got {w}");
        }
    }
    if !(1..=TREND_N_MAX).contains(&trend_n) {
        anyhow::bail!("--trend-n must be between 1 and {TREND_N_MAX}, got {trend_n}");
    }
    Ok(EstimationConfig {
        shrinkage,
        recency,
        recent_form_weight,
        trend_n,
        jockey_recent_form_weight: jockey_form_weight,
        running_style_weight: None,
        jockey_venue_weight: affinity.jockey_venue,
        jockey_distance_weight: affinity.jockey_distance,
        jockey_horse_combo_weight: affinity.jockey_horse_combo,
        horse_venue_weight: affinity.horse_venue,
        win_power,
        place_show_power,
        impute_missing_factors,
    })
}

#[cfg(test)]
mod config_override_tests {
    use super::*;

    /// フラグ未指定なら production 既定（m=10 / γ=1.25 / place_show=2.0 / impute=true）のまま（#282）。
    #[test]
    fn no_flags_keeps_production_defaults() {
        let c = production_config_with_overrides(None, None).unwrap();
        assert_eq!(c.shrinkage.unwrap().pseudo_count, 10.0);
        assert_eq!(c.win_power, Some(1.25));
        // 上書き対象外のフィールドは production のまま。
        assert_eq!(c.place_show_power, Some(2.0));
        assert!(c.impute_missing_factors);
    }

    /// 指定フラグだけを上書きし、他フィールドは production を維持する（#282）。
    #[test]
    fn overrides_only_given_fields() {
        let c = production_config_with_overrides(Some(20.0), Some(3.0)).unwrap();
        assert_eq!(c.shrinkage.unwrap().pseudo_count, 20.0);
        assert_eq!(c.win_power, Some(3.0));
        assert_eq!(c.place_show_power, Some(2.0), "place_show は据え置き");
        assert!(c.impute_missing_factors, "impute は据え置き");
    }

    /// 0/負/非有限は usage エラーで弾く（有限正数のみ許可）。
    #[test]
    fn rejects_non_positive_or_non_finite() {
        assert!(production_config_with_overrides(Some(0.0), None).is_err());
        assert!(production_config_with_overrides(Some(-1.0), None).is_err());
        assert!(production_config_with_overrides(None, Some(0.0)).is_err());
        assert!(production_config_with_overrides(None, Some(f64::NAN)).is_err());
    }
}

#[cfg(test)]
mod feature_dump_tests {
    use super::*;
    use chrono::NaiveDate;
    use paddock_domain::{HorseFactors, RateTriple};

    fn empty_factors() -> HorseFactors {
        HorseFactors {
            course_gate: None,
            horse_surface: None,
            horse_distance: None,
            jockey_surface: None,
            trainer_surface: None,
            horse_track_condition: None,
            jockey_venue: None,
            jockey_distance: None,
            jockey_horse_combo: None,
            horse_venue: None,
            recent_form: None,
            weight_carried: None,
            jockey_recent_form: None,
            running_style: None,
        }
    }

    /// ヘッダの列数が不変条件 [`FEATURE_DUMP_COLUMNS`] と一致すること（列追加時の更新漏れ検知）。
    #[test]
    fn header_has_expected_column_count() {
        assert_eq!(
            FEATURE_DUMP_HEADER.split('\t').count(),
            FEATURE_DUMP_COLUMNS
        );
    }

    /// 行生成の列数がヘッダと一致し、欠落（factor 全 None・signal None）は空セル、ラベルは実値を
    /// 文字列で運ぶこと（popularity が値で出る正例 + 欠落→空セルの 0 埋め無しを同時に担保）。
    #[test]
    fn row_cells_match_header_and_render_missing_as_empty() {
        let row = FeatureRow {
            race_id: "2026-1-nakayama-1-1R".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            horse_num: 7,
            factors: empty_factors(),
            model_win: 0.2,
            model_place: 0.3,
            model_show: 0.4,
            finishing_position: Some(1),
            win_odds: Some(4.0),
            popularity: Some(3),
        };
        let cells = feature_row_cells(&row);
        assert_eq!(cells.len(), FEATURE_DUMP_COLUMNS);
        assert_eq!(cells[0], "2026-1-nakayama-1-1R");
        assert_eq!(cells[2], "7");
        // factor 40 セル（cells[3..43]、#350 で 6→10 factor）は全欠落で空。
        assert!(
            cells[3..43].iter().all(String::is_empty),
            "欠落 factor は空セル"
        );
        // signal4（cells[43..47]、running_style 含む 4 列）も欠落で空。
        assert!(cells[43..47].iter().all(String::is_empty));
        // 内蔵モデル予測3（cells[47..50]）は必ず実値。
        assert_eq!(&cells[47..50], ["0.2", "0.3", "0.4"]);
        // ラベルは実値（finishing_position=1, win_odds=4.0→"4", popularity=3）。
        assert_eq!(cells[50], "1");
        assert_eq!(cells[51], "4");
        assert_eq!(cells[52], "3");
    }

    /// 実値を持つ factor は (win,place,show,starts) の 4 セルに展開され、欠落ラベルは空になること。
    #[test]
    fn row_cells_render_present_factor_stats() {
        let mut factors = empty_factors();
        factors.horse_surface = Some(FactorStat {
            rate: RateTriple {
                win: 0.3,
                place: 0.4,
                show: 0.5,
            },
            starts: 10,
        });
        // running_style に実値を入れ、signal4 の末尾セル(cells[46])に正しい位置で載ることを確認する
        // （#329 Phase1・列順デグレ検出。None 経路は別テストで担保）。
        factors.running_style = Some(0.75);
        let row = FeatureRow {
            race_id: "r".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            horse_num: 1,
            factors,
            model_win: 0.1,
            model_place: 0.2,
            model_show: 0.3,
            finishing_position: None,
            win_odds: None,
            popularity: None,
        };
        let cells = feature_row_cells(&row);
        // horse_surface は course_gate(3..7) の次の cells[7..11]。
        assert_eq!(&cells[7..11], ["0.3", "0.4", "0.5", "10"]);
        // running_style は signal4 の末尾セル cells[46]（jockey_recent_form の直後）。
        assert_eq!(cells[46], "0.75");
        // 内蔵モデル予測（cells[47..50]）は欠落しない。
        assert_eq!(&cells[47..50], ["0.1", "0.2", "0.3"]);
        // 欠落ラベルは空セル。
        assert_eq!(cells[50], "");
        assert_eq!(cells[51], "");
        assert_eq!(cells[52], "");
    }

    /// IO 本体 `write_feature_dump` が「ヘッダ行 + 各行 = `feature_row_cells` の TSV 連結」を出力し、
    /// 余計な行を足さないこと（出力契約の end-to-end 回帰固定）。tempfile 依存を足さず temp_dir を使う。
    #[test]
    fn write_feature_dump_emits_header_then_rows() {
        let row = FeatureRow {
            race_id: "2026-1-nakayama-1-1R".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            horse_num: 7,
            factors: empty_factors(),
            model_win: 0.2,
            model_place: 0.3,
            model_show: 0.4,
            finishing_position: Some(1),
            win_odds: Some(4.0),
            popularity: Some(3),
        };
        let path =
            std::env::temp_dir().join(format!("paddock_dump_test_{}.tsv", std::process::id()));
        write_feature_dump(path.to_str().unwrap(), std::slice::from_ref(&row)).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let mut lines = content.lines();
        assert_eq!(lines.next().unwrap(), FEATURE_DUMP_HEADER);
        assert_eq!(lines.next().unwrap(), feature_row_cells(&row).join("\t"));
        assert!(lines.next().is_none(), "ヘッダ + 1 行のみのはず");
    }
}
