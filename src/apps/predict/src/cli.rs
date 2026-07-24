use chrono::NaiveDate;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-predict",
    about = "1 日分のレースを対話的に予想し、買い目と収支を記録する",
    version
)]
pub struct Cli {
    /// 対象開催日（YYYY-MM-DD、例: 2026-06-01）。
    #[arg(long)]
    pub date: NaiveDate,

    /// 初期予算（円、例: 10000）。新規セッションの開始時のみ必須。
    /// `--resume` / `--summary` では保存済みセッションの値を使うため不要。
    #[arg(long)]
    pub budget: Option<u64>,

    /// 1 レースあたりの予算（円、軸流しポートフォリオの上限）。実上限は min(race_budget, 残高)。
    #[arg(long, default_value_t = 5000)]
    pub race_budget: u64,

    /// 中断した同日セッションを保存済みの残高から再開する。
    #[arg(long, conflicts_with_all = ["summary", "settle"])]
    pub resume: bool,

    /// 同日セッションの収支サマリと買い目明細を表示して終了する（読み取り専用）。
    #[arg(long)]
    pub summary: bool,

    /// レース確定後の事後精算。netkeiba の確定払戻で購入済み買い目の payout を自動セットし、
    /// セッションの収支・回収率を更新する（冪等。未確定レースはスキップ）。
    #[arg(long, conflicts_with = "summary")]
    pub settle: bool,

    /// 各レースで上位馬の予想根拠（条件別成績・前走サマリ）を表示する（#274）。
    /// 既定オフ。確率テーブル・買い目・確率値は本フラグの有無で一切変わらない。
    #[arg(long)]
    pub explain: bool,

    /// 全レースを非対話でスキップ（s 相当）扱いし、予想・買い目推奨だけを表示して流す（#479）。
    /// 馬場はデフォルト（記録済み→直前入力→確定値）を採用し、入力プロンプトを出さず値を表示する。
    /// 標準入力を一切読まないため、python ワンライナーでの `s` 連打パイプが不要になる。
    /// 買い目（bet_records）は記録しない。ただし馬場条件は #80 に従い対話時と同様に保存しうる
    /// （デフォルト値が未記録レースの記録と異なるとき保存が走る＝「どの馬場前提で予想したか」を再現可能に）。
    /// `--summary` / `--settle` は run_session を経由しないため排他とする（誤用を無視でなくエラーに）。
    #[arg(long, conflicts_with_all = ["summary", "settle"])]
    pub skip_all: bool,
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn skip_all_defaults_to_false() {
        let cli = Cli::parse_from(["paddock-predict", "--date", "2026-07-22"]);
        assert!(!cli.skip_all);
    }

    #[test]
    fn skip_all_flag_is_parsed() {
        let cli = Cli::parse_from(["paddock-predict", "--date", "2026-07-22", "--skip-all"]);
        assert!(cli.skip_all);
    }

    #[test]
    fn skip_all_conflicts_with_summary() {
        let res = Cli::try_parse_from([
            "paddock-predict",
            "--date",
            "2026-07-22",
            "--skip-all",
            "--summary",
        ]);
        assert!(res.is_err(), "--skip-all と --summary は排他であるべき");
    }

    #[test]
    fn skip_all_conflicts_with_settle() {
        let res = Cli::try_parse_from([
            "paddock-predict",
            "--date",
            "2026-07-22",
            "--skip-all",
            "--settle",
        ]);
        assert!(res.is_err(), "--skip-all と --settle は排他であるべき");
    }
}
