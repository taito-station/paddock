use clap::Parser;
use paddock_domain::{RaceId, Venue};
use paddock_use_case::{build_race_ids, paddock_race_id_from_netkeiba};

#[derive(Debug, Parser)]
#[command(
    name = "paddock-fetch-card",
    about = "netkeiba から当日の出馬表と単勝オッズを取得して race_cards/horse_entries/race_odds に保存する",
    version
)]
pub struct Cli {
    /// netkeiba の race_id（12 桁）。指定した場合 --year 等は不要。
    pub race_id: Option<String>,

    /// 開催年（race_id を使わず構成要素から組み立てる場合）。
    #[arg(long)]
    pub year: Option<u32>,

    /// 開催場（slug もしくは日本語: tokyo / 東京 等）。
    #[arg(long)]
    pub venue: Option<String>,

    /// 開催回。
    #[arg(long)]
    pub round: Option<u32>,

    /// 開催日次。
    #[arg(long)]
    pub day: Option<u32>,

    /// レース番号（R）。
    #[arg(long)]
    pub race: Option<u32>,

    /// 取得済みのカードを再取得して上書きする。
    #[arg(long)]
    pub force: bool,

    /// netkeiba へのリクエスト間隔（ミリ秒）。未指定ならスクレイパ既定値。
    #[arg(long)]
    pub interval: Option<u64>,
}

impl Cli {
    /// CLI 引数から `(netkeiba 12 桁 race_id, paddock RaceId)` を確定する。
    ///
    /// 位置引数 `race_id`（12 桁）が最優先。無ければ `--year/--venue/--round/--day/--race`
    /// 全指定から組み立てる。どちらも満たさなければエラー。
    pub fn resolve_race_id(&self) -> anyhow::Result<(String, RaceId)> {
        if let Some(id) = &self.race_id {
            let race_id = paddock_race_id_from_netkeiba(id)?;
            return Ok((id.clone(), race_id));
        }

        match (self.year, &self.venue, self.round, self.day, self.race) {
            (Some(year), Some(venue), Some(round), Some(day), Some(race)) => {
                let venue = Venue::try_from(venue.as_str())?;
                let (netkeiba, race_id) = build_race_ids(year, venue, round, day, race)?;
                Ok((netkeiba, race_id))
            }
            _ => anyhow::bail!(
                "race_id（12 桁）か --year/--venue/--round/--day/--race の全指定が必要です"
            ),
        }
    }
}
