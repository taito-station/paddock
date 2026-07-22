//! 監視ループ基盤（#459）。predict-watch / odds-collect が共有する「発走前レースを定期スイープし、
//! 全レース発走で自動終了する」骨格をここに 1 本化する。
//!
//! ## 共有する不変条件
//!
//! - **発走状態判定（[`classify`]）**: `races_by_date` は発走前レースを race_cards 由来で
//!   `track_condition=NULL`・`results` 空として返す（fact-check 済みの不変条件）。よって
//!   `has_result`（track_condition あり or results 非空）は「成績取込が済んだ＝確実に過去のレース」の
//!   早期シグナル。成績取込前でも発走済みになる通常遷移（発走直後）は `now > post` 側が捕捉する。
//! - **windowed / windowless の分岐**: predict-watch は「発走まで残り `window` 以内」だけを Due にする
//!   （窓あり）。odds-collect は終日収集で窓が無い（発走前なら常に Due）。この差は [`classify`] の
//!   `window: Option<Duration>` 1 個で表現する（`Some(w)`=windowed / `None`=windowless）。
//! - **防御チェック（[`count_started_before_post`]）**: 発走前（`now <= post`）なのに結果取込済みの
//!   レースは上記不変条件が崩れた兆候。放置すると Started 誤判定で監視が無言 no-op 化するため、両 app で
//!   検出・警告する（#459 以前は predict-watch のみにあり odds-collect に無い非対称だった）。

use chrono::{DateTime, Duration, Local, NaiveTime, Offset, TimeZone};
use paddock_domain::Race;

mod driver;
pub use driver::{Sweeper, run_monitor_loop};

/// `now` 時点でのレースの発走状態（windowed / windowless 共通）。
///
/// windowless（odds-collect）では `NotYet` は生じない（窓が無く発走前は必ず `Due`）。継続判定
/// （[`should_continue`]）は `Due | NotYet` を「発走前が残っている」として同一に扱う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceStatus {
    /// 発走前で対象（windowed=窓内 / windowless=発走前すべて）→ オッズ再取得の対象。
    Due,
    /// 発走前だが窓より先（windowed のみ）→ まだ対象外（次スイープ以降に Due 化）。
    NotYet,
    /// 発走済み（結果取込済み or 発走時刻超過）→ 対象外。
    Started,
    /// 発走時刻不明（post_time 無し）→ 判定不能、対象外。
    Unknown,
}

/// `now` 時点でのレース発走状態を判定する純関数（windowed / windowless 共通・単体テスト対象）。
///
/// - post_time 無し → `Unknown`
/// - 結果取込済み（`has_result`）または発走時刻超過（`now > post`）→ `Started`
/// - 発走前（`now <= post`）:
///   - `window = Some(w)`（windowed）: 残り `post - now` が `w` 以内なら `Due`、それより先は `NotYet`
///   - `window = None`（windowless）: 発走前なら常に `Due`（窓概念なし・終日収集）
pub fn classify(
    now: NaiveTime,
    post_time: Option<NaiveTime>,
    has_result: bool,
    window: Option<Duration>,
) -> RaceStatus {
    let Some(post) = post_time else {
        return RaceStatus::Unknown;
    };
    if has_result || now > post {
        return RaceStatus::Started;
    }
    // ここで now <= post。発走まで (post - now)。
    match window {
        // windowed: 窓内なら Due、窓の外は NotYet。
        Some(w) if post - now > w => RaceStatus::NotYet,
        // windowed の窓内、または windowless（窓なし）は発走前すべて Due。
        _ => RaceStatus::Due,
    }
}

/// 監視を継続すべきか（発走前のレースが残っているか）を判定する純関数（単体テスト対象）。
/// `Due` か `NotYet` が 1 つでもあれば継続、無ければ終了。
pub fn should_continue(statuses: &[RaceStatus]) -> bool {
    statuses
        .iter()
        .any(|s| matches!(s, RaceStatus::Due | RaceStatus::NotYet))
}

/// 結果取込済み（＝確実に発走済み）か。`races_by_date` の不変条件（発走前＝race_cards 由来で
/// track_condition=NULL・results 空）に依存した早期シグナル（crate docs 参照）。
pub fn has_result(race: &Race) -> bool {
    race.track_condition.is_some() || !race.results.is_empty()
}

/// JST(+09:00) を秒で表したオフセット。post_time は JST 起算のため、判定はこのオフセットを前提とする。
const JST_OFFSET_SECS: i32 = 9 * 3600;

/// 実行環境が発走状態判定の前提（当日・JST）を満たすか点検し、外れていれば警告を出す（#459・共通化）。
///
/// 発走状態は実行マシンの現在時刻と post_time の「時刻」だけで判定するため、(1) 当日以外の date、
/// (2) JST 以外の TZ では判定が無意味になる。誤用に早期に気づけるよう起動時に 1 度だけ呼ぶ。
/// `kind` は用途語（predict-watch=「発走状態」/ odds-collect=「収集対象」）で、警告文言を出し分ける。
/// `now_local` は呼び出し側の現在時刻（テスト時は固定値を注入できるよう引数で受ける）。
pub fn warn_if_not_today_jst<Tz: TimeZone>(
    date: chrono::NaiveDate,
    now_local: DateTime<Tz>,
    kind: &str,
) where
    Tz::Offset: std::fmt::Display,
{
    let today = now_local.date_naive();
    if date != today {
        println!(
            "⚠ --date {date} は本日（{today}）と異なります。発走状態は現在時刻と post_time の時刻のみで \
             判定するため、当日以外の指定では{kind}判定が正しく機能しません。",
        );
    }
    let tz_offset = now_local.offset().fix().local_minus_utc();
    if tz_offset != JST_OFFSET_SECS {
        // 半端な TZ（例 +05:30）も正しく出せるよう ±HH:MM 表記にする。
        let sign = if tz_offset < 0 { '-' } else { '+' };
        let abs = tz_offset.abs();
        println!(
            "⚠ 実行マシンのタイムゾーンが JST(+09:00) ではありません（現在 UTC{sign}{:02}:{:02}）。\
             post_time は JST 起算のため、{kind}判定がオフセットぶんずれます。JST マシンで実行してください。",
            abs / 3600,
            (abs % 3600) / 60,
        );
    }
}

/// ローカル現在時刻で [`warn_if_not_today_jst`] を呼ぶ薄いラッパ（実運用の入口）。
pub fn warn_if_not_today_jst_now(date: chrono::NaiveDate, kind: &str) {
    warn_if_not_today_jst(date, Local::now(), kind);
}

/// 「発走前（`now <= post`）なのに結果取込済み」のレース件数を数える純関数（#459・防御チェック）。
///
/// この状態は `races_by_date` の不変条件（発走前＝track_condition=NULL）が崩れた兆候。放置すると
/// `classify` が `Started` 誤判定して監視が無言 no-op 化するため、呼び出し側は 1 件以上で警告する。
/// `slots`・アクセサ（post_time / has_result）はジェネリックにして predict-watch / odds-collect の
/// 双方の Slot 型に効かせる（従来 predict-watch のみにあった防御を odds-collect にも共通化）。
pub fn count_started_before_post<S>(
    slots: &[S],
    now: NaiveTime,
    post_time: impl Fn(&S) -> Option<NaiveTime>,
    has_result: impl Fn(&S) -> bool,
) -> usize {
    slots
        .iter()
        .filter(|s| has_result(s) && post_time(s).is_some_and(|p| now <= p))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    // --- classify: 共通（windowed / windowless で同一） ---

    #[test]
    fn unknown_when_no_post_time() {
        assert_eq!(
            classify(t(15, 0), None, false, Some(Duration::minutes(40))),
            RaceStatus::Unknown
        );
        assert_eq!(classify(t(15, 0), None, false, None), RaceStatus::Unknown);
    }

    #[test]
    fn started_when_result_present() {
        // 結果取込済みは発走前の時刻でも Started（windowed / windowless とも）。
        assert_eq!(
            classify(t(14, 0), Some(t(15, 0)), true, Some(Duration::minutes(40))),
            RaceStatus::Started
        );
        assert_eq!(
            classify(t(14, 0), Some(t(15, 0)), true, None),
            RaceStatus::Started
        );
    }

    #[test]
    fn started_when_now_past_post() {
        assert_eq!(
            classify(t(15, 1), Some(t(15, 0)), false, Some(Duration::minutes(40))),
            RaceStatus::Started
        );
        assert_eq!(
            classify(t(15, 1), Some(t(15, 0)), false, None),
            RaceStatus::Started
        );
    }

    // --- classify: windowed（predict-watch 相当・窓 40 分） ---

    #[test]
    fn windowed_due_within_window_inclusive_boundary() {
        let w = Some(Duration::minutes(40));
        // 残り 40 分ちょうどは窓内（境界を含む）。
        assert_eq!(
            classify(t(14, 20), Some(t(15, 0)), false, w),
            RaceStatus::Due
        );
        // 残り 1 分も Due。
        assert_eq!(
            classify(t(14, 59), Some(t(15, 0)), false, w),
            RaceStatus::Due
        );
        // 発走時刻ちょうど（残り 0 分）も発走前扱いで Due。
        assert_eq!(
            classify(t(15, 0), Some(t(15, 0)), false, w),
            RaceStatus::Due
        );
    }

    #[test]
    fn windowed_not_yet_when_outside_window() {
        // 残り 41 分は窓の外。
        assert_eq!(
            classify(
                t(14, 19),
                Some(t(15, 0)),
                false,
                Some(Duration::minutes(40))
            ),
            RaceStatus::NotYet
        );
    }

    // --- classify: windowless（odds-collect 相当・窓なし） ---

    #[test]
    fn windowless_collect_when_before_post_regardless_of_distance() {
        // 窓概念が無いので、発走まで何分でも（早朝でも直前でも）発走前なら Due。
        assert_eq!(
            classify(t(9, 0), Some(t(15, 0)), false, None),
            RaceStatus::Due
        );
        assert_eq!(
            classify(t(14, 59), Some(t(15, 0)), false, None),
            RaceStatus::Due
        );
        // 発走時刻ちょうど（残り 0 分）も発走前扱いで Due。
        assert_eq!(
            classify(t(15, 0), Some(t(15, 0)), false, None),
            RaceStatus::Due
        );
        // windowless は NotYet を決して生まない。
        assert_ne!(
            classify(t(0, 1), Some(t(23, 59)), false, None),
            RaceStatus::NotYet
        );
    }

    #[test]
    fn should_continue_while_due_or_not_yet_remains() {
        use RaceStatus::*;
        assert!(should_continue(&[Started, Due, Started]));
        assert!(should_continue(&[Started, NotYet]));
        // 全て発走済み or 不明なら終了。
        assert!(!should_continue(&[Started, Started, Unknown]));
        assert!(!should_continue(&[Unknown]));
        // 空（その日に開催なし）も終了。
        assert!(!should_continue(&[]));
    }

    #[test]
    fn count_started_before_post_counts_only_invariant_breaks() {
        // (post_time, has_result) を持つミニ slot でアクセサをテストする。
        struct S(Option<NaiveTime>, bool);
        let now = t(14, 0);
        let slots = vec![
            S(Some(t(15, 0)), true),  // 発走前 + 結果あり → 不変条件破れ（カウント対象）
            S(Some(t(13, 0)), true),  // 発走後 + 結果あり → 通常の Started（対象外）
            S(Some(t(15, 0)), false), // 発走前 + 結果なし → 正常な Due（対象外）
            S(None, true),            // post_time 不明（対象外）
        ];
        assert_eq!(count_started_before_post(&slots, now, |s| s.0, |s| s.1), 1);
    }

    #[test]
    fn warn_helpers_do_not_panic() {
        // 出力（println）は検証しないが、JST/非JST・当日/非当日で panic しないことを担保する。
        let jst = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = jst.with_ymd_and_hms(2026, 7, 22, 12, 0, 0).unwrap();
        warn_if_not_today_jst(now.date_naive(), now, "発走状態");
        // 非当日・非 JST（+05:30）でも panic しない。
        let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
        let now2 = ist.with_ymd_and_hms(2026, 7, 21, 12, 0, 0).unwrap();
        warn_if_not_today_jst(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 22).unwrap(),
            now2,
            "収集対象",
        );
    }
}
