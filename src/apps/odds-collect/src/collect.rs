use chrono::{NaiveDate, NaiveTime};
use monitor_loop::{RaceStatus, Sweeper, has_result, run_monitor_loop, warn_if_not_today_jst_now};
use paddock_domain::Race;

use crate::cli::Cli;
use crate::setup::App;

/// 発走時刻付きのレース 1 件（races_by_date の Race ＋ race_card の post_time）。
struct Slot {
    race: Race,
    post_time: Option<NaiveTime>,
}

/// 指定日の全レースを取得し、各レースの post_time（race_card 由来）を引き当てる。
///
/// #459 で per-race `race_card`（N+1）を日付一括クエリ（post_time）に置き換えた。races_by_date の
/// 各レースに対し一括マップから引く（マップに無い＝未保存 NULL は None＝旧 per-card 経路で
/// `card.post_time` が None だったのと同一集合になる）。
async fn load_slots(app: &App, date: NaiveDate) -> anyhow::Result<Vec<Slot>> {
    let races = app.interactor.races_by_date(date).await?;
    let post_times = app.interactor.post_times_by_date(date).await?;
    let mut slots = Vec::with_capacity(races.len());
    for race in races {
        let post_time = post_times.get(&race.race_id).copied();
        slots.push(Slot { race, post_time });
    }
    Ok(slots)
}

/// レース表示ラベル（例: `函館10R 15:35`）。発走時刻不明なら `--:--`。
fn race_label(slot: &Slot) -> String {
    let post = slot
        .post_time
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".to_string());
    format!(
        "{}{}R {}",
        slot.race.venue.as_jp(),
        slot.race.race_num,
        post
    )
}

/// 1 スイープ: 発走前レースの単複オッズを再取得して新スナップショットを保存する。
async fn sweep(app: &App, slots: &[Slot], statuses: &[RaceStatus], now: NaiveTime) {
    let due: Vec<&Slot> = slots
        .iter()
        .zip(statuses)
        .filter(|(_, st)| **st == RaceStatus::Due)
        .map(|(s, _)| s)
        .collect();
    let unknown = statuses
        .iter()
        .filter(|st| **st == RaceStatus::Unknown)
        .count();

    println!(
        "── {} 収集スイープ: 対象 {} レース（単複のみ・新スナップショット保存）",
        now.format("%H:%M"),
        due.len(),
    );
    if unknown > 0 {
        println!(
            "   ※ 発走時刻不明（post_time 無し）{unknown} レースは対象外。当日 fetch-card 済みか確認。"
        );
    }

    let (mut ok, mut miss) = (0u32, 0u32);
    for slot in &due {
        let rid = &slot.race.race_id;
        match app.odds.refresh_win_place_odds(rid).await {
            Ok(Some(o)) => {
                ok += 1;
                println!("  ✓ {}: 単勝 {} 頭 保存", race_label(slot), o.win.len());
            }
            Ok(None) => {
                miss += 1;
                println!("  ・ {}: 未公開/失敗、スキップ", race_label(slot));
            }
            // refresh_win_place_odds は現状スクレイプ失敗を Ok(None) に畳むため Err は来ないが、
            // 将来の戻り値変更（DB エラー伝播等）に備えて防御的に握る。
            Err(e) => {
                miss += 1;
                println!("  ! {}: 取得エラー: {e}", race_label(slot));
            }
        }
    }
    if !due.is_empty() {
        println!("   → 保存 {ok} / スキップ {miss}");
    }
}

/// 収集ループを駆動する [`Sweeper`]（#459）。共通の骨格（[`run_monitor_loop`]）へ odds-collect 固有の
/// slots ロード（post_time のみ）・単複収集スイープ・windowless 設定を注入する。
struct CollectSweeper<'a> {
    app: &'a App,
    cli: &'a Cli,
}

impl Sweeper for CollectSweeper<'_> {
    type Slot = Slot;

    async fn load_slots(&self) -> anyhow::Result<Vec<Slot>> {
        load_slots(self.app, self.cli.date).await
    }

    fn post_time(slot: &Slot) -> Option<NaiveTime> {
        slot.post_time
    }

    fn has_result(slot: &Slot) -> bool {
        has_result(&slot.race)
    }

    /// 終日収集なので窓は無い（発走前なら常に Due）。
    fn window(&self) -> Option<chrono::Duration> {
        None
    }

    fn date(&self) -> NaiveDate {
        self.cli.date
    }

    fn once(&self) -> bool {
        self.cli.once
    }

    fn interval_minutes(&self) -> u64 {
        self.cli.interval
    }

    fn finish_noun(&self) -> &str {
        "収集"
    }

    fn fetch_card_hint(&self) -> &str {
        "当日 fetch-card 済みか確認してください。"
    }

    async fn sweep(&mut self, slots: &[Slot], statuses: &[RaceStatus], now: NaiveTime) {
        sweep(self.app, slots, statuses, now).await;
    }
}

/// 収集ループ。発走前のレースが残っている間スイープを繰り返し、全レース発走で自動終了する。
///
/// 非 once の常駐モードでも全 Unknown（post_time 全欠落）は終了する: 当日 fetch-card 済みが前提で、
/// post_time が恒久的に無い状況で待ち続けると無限ループになるため（[`run_monitor_loop`] が終了判定する）。
/// カード未取得での誤起動は終了メッセージで気づける（fetch-card 後に再起動する運用）。
pub async fn run(app: &App, cli: &Cli) -> anyhow::Result<()> {
    // 連続再取得（busy loop）で netkeiba を叩き続けないよう、間隔は 1 分以上（cli の
    // value_parser=range(1..) で parse 時に強制。DB 接続前に弾くため実行時チェックは不要）。

    // 発走状態は実行マシンの現在時刻と post_time の「時刻」だけで判定するため、当日以外の date や
    // JST 以外の TZ では判定が無意味になる。誤用に早期に気づけるよう起動時に警告する（#459 で共通化）。
    warn_if_not_today_jst_now(cli.date, "収集対象");

    let mut sweeper = CollectSweeper { app, cli };
    run_monitor_loop(&mut sweeper).await
}
