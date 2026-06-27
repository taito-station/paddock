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
    },
    /// 古い race_odds_snapshots を保持期間でパージする（#234）。最新キャッシュ race_odds は消さない。
    /// cutoff = 実行日(UTC) − months。fetched_at の日付が cutoff より前の行を削除する。
    PurgeSnapshots {
        /// 保持月数（これより古い fetched_at の行を削除）。既定 12。#218 が必要とする直近 3〜6 ヶ月を
        /// 割らないよう下限 1（0 は全削除に近く危険なため弾く）。
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
}
