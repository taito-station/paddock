use std::time::Duration as StdDuration;

use chrono::{Local, NaiveDate, NaiveTime, Offset};
use paddock_domain::Race;

use crate::cli::Cli;
use crate::setup::App;

/// `now` 時点での収集対象状態（predict-watch の `RaceStatus` を window 概念なしに単純化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectStatus {
    /// 発走前 → 単複オッズを収集する。
    Collect,
    /// 発走済み（結果取込済み or 発走時刻超過）→ 対象外。
    Started,
    /// 発走時刻不明（post_time 無し）→ 判定不能、対象外。
    Unknown,
}

/// `now` 時点の収集対象状態を判定する純関数（単体テスト対象）。終日収集なので窓は無い。
/// - post_time 無し → `Unknown`
/// - 結果取込済み（`has_result`）または発走時刻超過（`now > post`）→ `Started`
/// - それ以外（発走前・`now <= post`）→ `Collect`
pub fn classify(now: NaiveTime, post_time: Option<NaiveTime>, has_result: bool) -> CollectStatus {
    let Some(post) = post_time else {
        return CollectStatus::Unknown;
    };
    if has_result || now > post {
        return CollectStatus::Started;
    }
    CollectStatus::Collect
}

/// 収集を継続すべきか（発走前のレースが残っているか）を判定する純関数（単体テスト対象）。
pub fn should_continue(statuses: &[CollectStatus]) -> bool {
    statuses.contains(&CollectStatus::Collect)
}

/// 結果取込済み（＝確実に発走済み）か。`races_by_date` は発走前レースを race_cards 由来で
/// track_condition=NULL・results 空として返すため、この経路では track_condition/results は
/// 「成績取込が済んだ＝確実に過去のレース」の早期シグナル（predict-watch と同じ不変条件）。
fn has_result(race: &Race) -> bool {
    race.track_condition.is_some() || !race.results.is_empty()
}

/// 発走時刻付きのレース 1 件（races_by_date の Race ＋ race_card の post_time）。
struct Slot {
    race: Race,
    post_time: Option<NaiveTime>,
}

/// 指定日の全レースを取得し、各レースの post_time（race_card 由来）を引き当てる。
async fn load_slots(app: &App, date: NaiveDate) -> anyhow::Result<Vec<Slot>> {
    let races = app.interactor.races_by_date(date).await?;
    let mut slots = Vec::with_capacity(races.len());
    for race in races {
        let post_time = app
            .interactor
            .race_card(&race.race_id)
            .await?
            .and_then(|c| c.post_time);
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
async fn sweep(app: &App, slots: &[Slot], statuses: &[CollectStatus], now: NaiveTime) {
    let due: Vec<&Slot> = slots
        .iter()
        .zip(statuses)
        .filter(|(_, st)| **st == CollectStatus::Collect)
        .map(|(s, _)| s)
        .collect();
    let unknown = statuses
        .iter()
        .filter(|st| **st == CollectStatus::Unknown)
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

/// 収集ループ。発走前のレースが残っている間スイープを繰り返し、全レース発走で自動終了する。
pub async fn run(app: &App, cli: &Cli) -> anyhow::Result<()> {
    // 連続再取得（busy loop）で netkeiba を叩き続けないよう、間隔は 1 分以上（cli の
    // value_parser=range(1..) で parse 時に強制。DB 接続前に弾くため実行時チェックは不要）。

    // 発走状態は実行マシンの現在時刻と post_time の「時刻」だけで判定するため、(1) 当日以外の date、
    // (2) JST 以外の TZ では判定が無意味になる。誤用に早期に気づけるよう起動時に警告する（predict-watch と同旨）。
    const JST_OFFSET_SECS: i32 = 9 * 3600;
    let now_local = Local::now();
    let today = now_local.date_naive();
    if cli.date != today {
        println!(
            "⚠ --date {} は本日（{today}）と異なります。発走状態は現在時刻と post_time の時刻のみで \
             判定するため、当日以外では収集対象判定が正しく機能しません。",
            cli.date,
        );
    }
    let tz_offset = now_local.offset().fix().local_minus_utc();
    if tz_offset != JST_OFFSET_SECS {
        let sign = if tz_offset < 0 { '-' } else { '+' };
        let abs = tz_offset.abs();
        println!(
            "⚠ 実行マシンのタイムゾーンが JST(+09:00) ではありません（現在 UTC{sign}{:02}:{:02}）。\
             post_time は JST 起算のため、収集対象判定がオフセットぶんずれます。JST マシンで実行してください。",
            abs / 3600,
            (abs % 3600) / 60,
        );
    }

    loop {
        // 継続中の一時的 DB エラーでプロセスを落とすと収集を取りこぼす。握って次スイープへ続行する
        //（--once 時のみ伝播して非ゼロ終了）。
        let slots = match load_slots(app, cli.date).await {
            Ok(s) => s,
            Err(e) if cli.once => return Err(e),
            Err(e) => {
                println!("⚠ レース一覧の取得に失敗（次スイープで再試行）: {e}");
                tokio::time::sleep(StdDuration::from_secs(cli.interval * 60)).await;
                continue;
            }
        };
        let now = Local::now().time();
        let statuses: Vec<CollectStatus> = slots
            .iter()
            .map(|s| classify(now, s.post_time, has_result(&s.race)))
            .collect();

        sweep(app, &slots, &statuses, now).await;

        // 発走前（Collect）が無ければ終了。非 once の常駐モードでも全 Unknown（post_time 全欠落）は
        // 終了する: 当日 fetch-card 済みが前提で、post_time が恒久的に無い状況で待ち続けると無限ループに
        // なるため。カード未取得での誤起動は下の警告メッセージで気づける（fetch-card 後に再起動する運用）。
        if !should_continue(&statuses) {
            if statuses.is_empty() {
                println!("── 収集終了: 本日（{}）は対象開催がありません。", cli.date);
            } else if statuses.iter().all(|s| *s == CollectStatus::Unknown) {
                println!(
                    "── 収集終了: 全レースで発走時刻（post_time）が不明です。当日 fetch-card 済みか確認してください。"
                );
            } else {
                println!("── 収集終了: 発走前のレースが残っていません。");
            }
            break;
        }
        if cli.once {
            println!("── --once 指定のため 1 スイープで終了します。");
            break;
        }
        tokio::time::sleep(StdDuration::from_secs(cli.interval * 60)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn unknown_when_no_post_time() {
        assert_eq!(classify(t(15, 0), None, false), CollectStatus::Unknown);
    }

    #[test]
    fn started_when_result_present() {
        // 結果取込済みは発走前の時刻でも Started（収集しない）。
        assert_eq!(
            classify(t(14, 0), Some(t(15, 0)), true),
            CollectStatus::Started
        );
    }

    #[test]
    fn started_when_now_past_post() {
        assert_eq!(
            classify(t(15, 1), Some(t(15, 0)), false),
            CollectStatus::Started
        );
    }

    #[test]
    fn collect_when_before_post_regardless_of_distance() {
        // 窓概念が無いので、発走まで何分でも（早朝でも直前でも）発走前なら Collect。
        assert_eq!(
            classify(t(9, 0), Some(t(15, 0)), false),
            CollectStatus::Collect
        );
        assert_eq!(
            classify(t(14, 59), Some(t(15, 0)), false),
            CollectStatus::Collect
        );
        // 発走時刻ちょうど（残り 0 分）も発走前扱いで Collect。
        assert_eq!(
            classify(t(15, 0), Some(t(15, 0)), false),
            CollectStatus::Collect
        );
    }

    #[test]
    fn should_continue_while_collect_remains() {
        use CollectStatus::*;
        assert!(should_continue(&[Started, Collect, Started]));
        assert!(should_continue(&[Collect]));
        // 全て発走済み or 不明なら終了。
        assert!(!should_continue(&[Started, Started, Unknown]));
        assert!(!should_continue(&[Unknown]));
        // 空（その日に開催なし）も終了。
        assert!(!should_continue(&[]));
    }
}
