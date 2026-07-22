use std::time::Duration as StdDuration;

use chrono::{Duration, Local, NaiveTime};

use crate::{RaceStatus, classify, count_started_before_post, should_continue};

/// 監視ループの app 固有部分を供給するトレイト（#459）。
///
/// ループ骨格（[`run_monitor_loop`]）は「slots ロード → 状態判定 → 防御チェック → sweep → 継続/終了判定
/// → interval sleep」の流れと、DB エラー握り・`--once` 伝播・終了メッセージ分岐を担う。app 固有の
/// 「何を Slot とするか」「1 スイープで何をするか」「windowed か windowless か」だけをこのトレイトで注入する。
///
/// windowed（predict-watch＝発走前後の窓）/ windowless（odds-collect＝終日）の差は [`Self::window`] の
/// `Option<Duration>` 1 個で表す。予算 override の初回チェック等、app 固有の per-sweep 前処理は
/// [`Self::sweep`] 内に閉じ込める（`&mut self` で状態を持てる）。
pub trait Sweeper {
    /// 1 レース分の作業単位（predict-watch / odds-collect でフィールドが異なる）。
    type Slot;

    /// 指定日の全 Slot を取得する（races_by_date ＋ post_time 等の一括引き当て）。
    /// DB エラーはここで `Err` にして返す（ループ側が握って次スイープへ続行、`--once` 時のみ伝播）。
    fn load_slots(&self) -> impl Future<Output = anyhow::Result<Vec<Self::Slot>>> + Send;

    /// Slot の発走時刻（race_card 由来。未取得は `None`）。状態判定に使う。
    fn post_time(slot: &Self::Slot) -> Option<NaiveTime>;

    /// Slot が結果取込済み（＝確実に発走済み）か。[`crate::has_result`] 由来。
    fn has_result(slot: &Self::Slot) -> bool;

    /// 先読み窓。`Some(w)`=windowed（発走まで `w` 以内だけ Due）/ `None`=windowless（発走前すべて Due）。
    fn window(&self) -> Option<Duration>;

    /// 対象日（YYYY-MM-DD）。終了メッセージや slots ロードに使う。
    fn date(&self) -> chrono::NaiveDate;

    /// 1 スイープだけで終了するか（`--once`）。
    fn once(&self) -> bool;

    /// スイープ間隔（分）。継続時にこの分だけ sleep する。
    fn interval_minutes(&self) -> u64;

    /// 1 スイープ本体。`statuses` は `slots` と同順の発走状態（ループが 1 度だけ算出して渡す）。
    /// app 固有の対象抽出（Due のみ）・オッズ再取得・EV/収集処理・per-sweep 前処理をここで行う。
    fn sweep(
        &mut self,
        slots: &[Self::Slot],
        statuses: &[RaceStatus],
        now: NaiveTime,
    ) -> impl Future<Output = ()> + Send;

    /// 終了メッセージのラベル語（predict-watch=「監視」/ odds-collect=「収集」）。
    fn finish_noun(&self) -> &str;

    /// 全レース post_time 不明で終了するときの fetch-card 案内文（app ごとに微差。既定は共通文言）。
    /// odds-collect は「当日 fetch-card 済み」を促す文言に上書きする。
    fn fetch_card_hint(&self) -> &str {
        "fetch-card 済みか確認してください。"
    }
}

/// 監視ループ骨格（#459・predict-watch / odds-collect 共通）。
///
/// 発走前のレースが残っている間スイープを繰り返し、全レース発走で自動終了する。継続監視中の一時的
/// DB エラーはプロセスを落とさず握って次スイープへ続行する（`--once` 時のみ伝播して非ゼロ終了）。
/// 発走前なのに結果取込済みのレース（[`count_started_before_post`]）を検出したら警告する（両 app 共通の防御）。
pub async fn run_monitor_loop<S: Sweeper>(sweeper: &mut S) -> anyhow::Result<()> {
    let date = sweeper.date();
    let once = sweeper.once();
    let interval = sweeper.interval_minutes();
    let window = sweeper.window();
    let noun = sweeper.finish_noun().to_string();
    let fetch_card_hint = sweeper.fetch_card_hint().to_string();

    loop {
        // 継続監視中の一時的 DB エラーでプロセスを落とすと取りこぼす。握って次スイープへ続行する
        //（--once 時のみ伝播して非ゼロ終了）。
        let slots = match sweeper.load_slots().await {
            Ok(s) => s,
            Err(e) if once => return Err(e),
            Err(e) => {
                println!("⚠ レース一覧の取得に失敗（次スイープで再試行）: {e}");
                tokio::time::sleep(StdDuration::from_secs(interval * 60)).await;
                continue;
            }
        };

        let now = Local::now().time();
        // 発走状態は 1 スイープ 1 回だけ算出し、sweep 表示と終了判定で共有する。
        let statuses: Vec<RaceStatus> = slots
            .iter()
            .map(|s| classify(now, S::post_time(s), S::has_result(s), window))
            .collect();

        // 防御: 発走前（now <= post）なのに結果取込済みのレースは、races_by_date の不変条件
        //（発走前＝race_cards 由来で track_condition=NULL）が崩れた兆候。放置すると Started 誤判定で
        // 監視が無言 no-op 化するため、検出したら警告する（#459 で両 app に共通化）。
        let started_before_post =
            count_started_before_post(&slots, now, S::post_time, S::has_result);
        if started_before_post > 0 {
            println!(
                "⚠ 発走前なのに結果取込済みのレースが {started_before_post} 件あります。発走状態判定の前提が \
                 崩れている可能性があり、対象から外れます（fetch-card / 成績取込の状態を確認してください）。"
            );
        }

        sweeper.sweep(&slots, &statuses, now).await;

        if !should_continue(&statuses) {
            if statuses.is_empty() {
                println!("── {noun}終了: 本日（{date}）は対象開催がありません。");
            } else if statuses.iter().all(|s| *s == RaceStatus::Unknown) {
                println!(
                    "── {noun}終了: 全レースで発走時刻（post_time）が不明です。{fetch_card_hint}"
                );
            } else {
                println!("── {noun}終了: 発走前のレースが残っていません。");
            }
            break;
        }
        if once {
            println!("── --once 指定のため 1 スイープで終了します。");
            break;
        }
        tokio::time::sleep(StdDuration::from_secs(interval * 60)).await;
    }
    Ok(())
}
