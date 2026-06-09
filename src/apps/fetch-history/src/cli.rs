use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "paddock-fetch-history",
    about = "netkeiba から当日出走馬の近走を取得して results に取り込む",
    version
)]
pub struct Cli {
    /// netkeiba の race_id（12 桁、複数可）。出馬表から各馬の horse_id を引いて近走を取得する。
    pub race_ids: Vec<String>,

    /// 出馬表をバイパスして近走を取得する horse_id（複数可）。race_id と併用可。
    #[arg(long = "horse-id")]
    pub horse_ids: Vec<String>,

    /// 取得後の pdf 成績行 horse_id backfill を抑制する（既定は自動実行）。
    #[arg(long = "no-backfill")]
    pub no_backfill: bool,
}
