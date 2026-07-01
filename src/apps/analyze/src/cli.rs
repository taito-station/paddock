use clap::{Parser, Subcommand};
use paddock_domain::TrackCondition;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-analyze",
    about = "Query JRA race statistics aggregated from parsed PDFs",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Stats for a single horse (overall, by surface, distance band, gate group, track condition).
    Horse {
        /// Horse name (Japanese, exact match).
        name: String,
    },
    /// Gate-position win/place rate at a specific course/distance/surface.
    Course {
        /// Venue name (例: 中山, 阪神, 東京).
        venue: String,
        /// Distance in meters (例: 2000).
        distance: u32,
        /// Surface: turf or dirt.
        surface: String,
    },
    /// Stats for a jockey (overall, by surface, by gate group).
    Jockey {
        /// Jockey name (Japanese, exact match).
        name: String,
    },
    /// Stats for a trainer (overall, by surface, by gate group).
    Trainer {
        /// Trainer name (Japanese, exact match).
        name: String,
    },
    /// Predict win/place/show probabilities for each horse in a race.
    /// win <= place <= show monotonicity is guaranteed; place/show are top-2 / top-3 probabilities
    /// (normalized to sum 2.0 / 3.0 across the field, then monotonized). See ADR 0007.
    Predict {
        /// Race ID（paddock 形式 `{年}-{回}-{場slug}-{日}-{R}R`、例: 2026-3-nakayama-8-1R）。
        race_id: String,
        /// 市場オッズ(単勝)ブレンドのモデル重み α [0,1]。未指定でモデルのみ、
        /// 指定すると最新オッズスナップショット(時刻制約なし)の implied 確率と (1-α) で
        /// ブレンドする（#72）。
        #[arg(long)]
        blend_alpha: Option<f64>,
        /// 当日の馬場状態（良/稍重/重/不良。稍/不 の略記も可）。指定すると各馬の
        /// 馬場状態別成績を factor に加える（#73）。出馬表 PDF に馬場状態は無いため
        /// 手で渡す。未指定なら馬場項なし。
        #[arg(long, value_parser = parse_track_condition)]
        track_condition: Option<TrackCondition>,
    },
    /// Backtest the prediction logic over finished races in a date range.
    /// Reproduces probability estimation with as-of stats (no leakage) and reports
    /// hit rate / expected payout rate / Brier / LogLoss.
    Backtest {
        /// 開始日 YYYY-MM-DD（含む）。
        #[arg(long)]
        from: String,
        /// 終了日 YYYY-MM-DD（含む）。
        #[arg(long)]
        to: String,
        /// 市場オッズ(単勝)ブレンドのモデル重み α [0,1]。未指定でモデルのみ、
        /// 指定すると当時オッズの implied 確率と (1-α) でブレンドする（#72）。
        #[arg(long)]
        blend_alpha: Option<f64>,
        /// ベイズ縮約の擬似カウント m（#75）。指定すると各 factor のレートを母集団 prior へ
        /// `(k·rate + m·prior)/(k + m)` で縮約する。未指定は縮約なし（現行挙動）。
        /// パラメータスイープ（5/10/20/50 等）で校正改善を比較するために使う。
        #[arg(long)]
        shrinkage_m: Option<f64>,
        /// リーセンシー重み付けの半減期（日, #75 Phase B）。指定すると馬の芝ダ・距離帯・馬場状態
        /// factor を直近成績ほど重く `0.5^(days_ago/half_life)` で時間減衰集計する。未指定は減衰なし。
        /// スイープ（30/60/90 等）で校正改善を比較する。
        #[arg(long)]
        recency_half_life: Option<f64>,
        /// 前走フォーム項の重みオーバーライド（#217）。未指定はデフォルト 0.25（FORM_WEIGHT）。
        /// スイープ（0.0/0.1/0.2/0.25/0.3/0.4/0.5 等）で最適重みを探す。
        #[arg(long)]
        recent_form_weight: Option<f64>,
        /// 直近 N 走トレンドの走数（#220）。重みは [1.0, 0.5, 0.25] 固定。
        /// `1`（デフォルト）= 前走のみ（現行挙動）。スイープ（1/2/3）で改善を比較する。
        #[arg(long, default_value_t = 1)]
        trend_n: u32,
        /// 騎手直近フォーム項の重みオーバーライド（#221）。未指定はデフォルト 0.25。
        /// スイープ（0.0/0.1/0.25/0.5/1.0 等）で最適重み（または棄却）を探す（ADR 0038）。
        #[arg(long)]
        jockey_form_weight: Option<f64>,
        /// win_prob 冪変換のγ（#246）。`win'_i ∝ win_i^γ` で再正規化し穴馬の 1 着過大評価を縮約する。
        /// 未指定は no-op（現行挙動）。スイープ（1.1/1.25/1.5/2.0 等。γ<1 は逆方向）で
        /// 単勝校正・人気帯校正・回収を比較する（ADR 0042）。
        #[arg(long)]
        win_power: Option<f64>,
        /// place/show スコア冪変換のγ（#283 / #258 Phase 2）。正規化前に `score^γ` でシャープ化し、
        /// 正規化＋単調化が招く分布の中央圧縮（本命の複勝過小評価・人気薄過大評価）を脱圧縮する。
        /// 未指定は no-op（現行挙動）。スイープ（1.25/1.5/2.0 等。γ<1 は逆方向）で複勝校正・人気帯
        /// 校正・複勝回収を比較する（trio/馬連/馬単は win_prob 由来で本フラグでは不変）。
        #[arg(long)]
        place_show_power: Option<f64>,
        /// 欠落 stat factor をレース内 field mean で補完する（#272 改善② / ADR 0057）。指定すると各 factor を
        /// 欠く馬に present 馬の縮約後レート平均（present<2 は prior）を代入し weight も数える。未指定は従来
        /// どおり欠落項を母数から落とす（drop）。純 resolution 改善（AUC/top1）と blended 非回帰を A/B するための
        /// フラグ。predict 本番（`EstimationConfig::production()`）は既定で有効。
        #[arg(long)]
        impute_missing_factors: bool,
        /// 学習型モデル評価ハーネス用の特徴量ダンプ出力先 TSV パス（#272 Phase A）。指定すると各
        /// 出走馬の素性（ブレンド・冪変換前）＋ラベル（確定着順・人気）＋当時市場単勝をリーク無しの
        /// walk-forward で書き出す。未指定は集計レポートのみ（既存挙動）。
        #[arg(long)]
        dump_features: Option<String>,
    },
    /// 古い race_odds_snapshots を保持期間でパージする（#234）。最新キャッシュ race_odds は消さない。
    /// cutoff = 実行日(UTC) − months。fetched_at の日付が cutoff より前の行を削除する。
    PurgeSnapshots {
        /// 保持月数（これより古い fetched_at の行を削除）。既定 12（#218 が必要とする直近 3〜6 ヶ月を
        /// 十分上回る）。下限ガードは 0 のみを弾く（0 は当日以降だけ保持＝ほぼ全削除で危険）。3〜6 ヶ月の
        /// 確保は「既定 12 を使う／運用で十分大きい months を指定する」前提で、ここでは 0 だけを防ぐ。
        #[arg(long, default_value_t = 12)]
        months: u32,
        /// 削除せず該当行数のみ表示する。
        #[arg(long)]
        dry_run: bool,
    },
}

/// clap 用: 馬場状態のパース。引数解析時に検証し、不正値は usage エラーとして報告する。
fn parse_track_condition(s: &str) -> Result<TrackCondition, String> {
    TrackCondition::try_from(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command};
    use paddock_domain::TrackCondition;

    /// `--track-condition` が value_parser 経由で enum に解決されること（略記含む）。
    #[test]
    fn predict_parses_track_condition_including_abbreviation() {
        let cli = Cli::try_parse_from([
            "paddock-analyze",
            "predict",
            "2026-1-tokyo-1-R1",
            "--track-condition",
            "稍",
        ])
        .unwrap();
        match cli.command {
            Command::Predict {
                track_condition, ..
            } => assert_eq!(track_condition, Some(TrackCondition::Good)),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// 不正値は実行前に usage エラー（value_parser での値検証）として弾かれること。
    #[test]
    fn predict_rejects_unknown_track_condition() {
        let err = Cli::try_parse_from([
            "paddock-analyze",
            "predict",
            "2026-1-tokyo-1-R1",
            "--track-condition",
            "泥",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    /// `--impute-missing-factors` は bool フラグで、未指定は false（drop）・指定で true（#272 改善②）。
    #[test]
    fn backtest_parses_impute_missing_factors_flag() {
        let parse = |extra: &[&str]| {
            let mut args = vec![
                "paddock-analyze",
                "backtest",
                "--from",
                "2025-01-01",
                "--to",
                "2025-01-31",
            ];
            args.extend_from_slice(extra);
            match Cli::try_parse_from(args).unwrap().command {
                Command::Backtest {
                    impute_missing_factors,
                    ..
                } => impute_missing_factors,
                other => panic!("unexpected command: {other:?}"),
            }
        };
        assert!(!parse(&[]), "未指定は drop（false）");
        assert!(parse(&["--impute-missing-factors"]), "指定で補完（true）");
    }
}
