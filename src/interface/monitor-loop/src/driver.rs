use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Local, NaiveTime};

use crate::{
    RaceStatus, classify, count_started_before_post, detect_sweep_gap, keep_awake, should_continue,
    should_stop_by_date,
};

/// wall-clock 期限を待つときの刻み幅。単発の長い sleep はホストのスリープを跨げないため、この
/// 間隔で現在時刻と期限を比べ直す（#568）。短くするほど復帰検知は速いが空回りが増えるので、
/// 最短スイープ間隔（1 分）に対して十分細かい 30 秒に置く。
const WAKE_CHECK_TICK: StdDuration = StdDuration::from_secs(30);

/// wall-clock の期限まで `tick` 刻みで待つ（#568）。
///
/// `tokio::time::sleep(長時間)` 1 回だとホストがスリープした際にタイマーが進まず、復帰後も
/// 「残り」を待ち続けて監視が無言で止まる。毎ティック現在時刻と期限を比べ直すことで、復帰時点で
/// 期限を過ぎていれば即座に抜ける＝次スイープが走る。
///
/// `tick` は実運用では [`WAKE_CHECK_TICK`] 固定。テストが実時間を待たずに「複数ティックを跨ぐ」
/// 経路を踏めるよう引数にしている。
async fn sleep_until_with_tick(deadline: DateTime<Local>, tick: StdDuration) {
    // 期限を過ぎている（差が負）なら to_std が Err になり、ループを抜ける＝即座に次スイープへ。
    while let Ok(remaining) = (deadline - Local::now()).to_std() {
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(remaining.min(tick)).await;
    }
}

/// 次スイープまで待ち、待機が想定間隔を大きく超えて途切れていたら警告する（#568）。
///
/// 沈黙＝正常に見える問題（スリープで監視が止まっていたのに何のログも残らない）を解消するため、
/// 実経過を測って [`detect_sweep_gap`] に判定させる。
async fn wait_next_sweep(interval_minutes: u64) {
    let started = Local::now();
    let deadline = started + Duration::minutes(i64::try_from(interval_minutes).unwrap_or(i64::MAX));
    sleep_until_with_tick(deadline, WAKE_CHECK_TICK).await;
    if let Some(minutes) = detect_sweep_gap(interval_minutes, Local::now() - started) {
        println!(
            "⚠ スイープが {minutes} 分途切れました（想定 {interval_minutes} 分間隔・ホストのスリープ/停止の可能性）。\
             直ちに再スイープします。途切れていた間に発走したレースは評価されていません。"
        );
    }
}

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
///
/// ホストのスリープを跨いでも監視が死なないよう、待機は wall-clock 期限で刻み（[`sleep_until_with_tick`]）、
/// 途切れを検知して警告し（[`wait_next_sweep`]）、日付を跨いだら終了する（[`should_stop_by_date`]・#568）。
pub async fn run_monitor_loop<S: Sweeper>(sweeper: &mut S) -> anyhow::Result<()> {
    let date = sweeper.date();
    let once = sweeper.once();
    let interval = sweeper.interval_minutes();
    let window = sweeper.window();
    let noun = sweeper.finish_noun().to_string();
    let fetch_card_hint = sweeper.fetch_card_hint().to_string();

    // 監視中はホストのアイドルスリープを抑止する（#568）。返り値を保持している間だけ有効で、
    // 監視の終了（正常/異常を問わず）で自動解放される。--once は単発なので確保しない。
    let _keep_awake = if once { None } else { keep_awake::acquire() };

    loop {
        // 継続監視中の一時的 DB エラーでプロセスを落とすと取りこぼす。握って次スイープへ続行する
        //（--once 時のみ伝播して非ゼロ終了）。
        let slots = match sweeper.load_slots().await {
            Ok(s) => s,
            Err(e) if once => return Err(e),
            Err(e) => {
                println!("⚠ レース一覧の取得に失敗（次スイープで再試行）: {e}");
                wait_next_sweep(interval).await;
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
        // 発走状態判定（classify）は時刻だけを見るため、日付を跨ぐと昨日のレースが再び「発走前」に
        // 見えて should_continue が永久に true を返す（#568 の「終了しない」主因）。wall-clock の
        // 日付で止める。判定はスイープ後に置き、1 スイープも走らない経路を作らない。
        if should_stop_by_date(date, Local::now().date_naive()) {
            println!(
                "── {noun}終了: 対象日（{date}）を過ぎました。発走状態は時刻のみで判定するため、日付を跨いだら終了します。"
            );
            break;
        }
        wait_next_sweep(interval).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    // --- sleep_until_with_tick: wall-clock 期限で刻んで待つ（#568） ---

    #[tokio::test]
    async fn returns_immediately_when_deadline_already_passed() {
        // スリープから復帰した直後の状況（期限をとうに過ぎている）。待たずに抜けて次スイープへ進む。
        let started = Local::now();
        sleep_until_with_tick(started - Duration::hours(4), StdDuration::from_millis(50)).await;
        assert!((Local::now() - started) < Duration::seconds(1));
    }

    #[tokio::test]
    async fn waits_until_deadline_across_multiple_ticks() {
        // 期限までティックを複数回跨いで待つ（実運用の 30 秒刻みを 20ms に縮めた等価経路）。
        let started = Local::now();
        let deadline = started + Duration::milliseconds(200);
        sleep_until_with_tick(deadline, StdDuration::from_millis(20)).await;
        let elapsed = Local::now() - started;
        // 期限より手前で抜けない（早すぎる再スイープを起こさない）。
        assert!(elapsed >= Duration::milliseconds(200), "elapsed={elapsed}");
        // かつ刻み待ちで大幅に伸びない。
        assert!(elapsed < Duration::seconds(5), "elapsed={elapsed}");
    }

    // --- run_monitor_loop: 日付跨ぎで終了する（#568） ---

    /// DB を持たない最小の [`Sweeper`]。発走前（Due）のレースが残り続ける状況を作り、
    /// 「時刻軸では終われない」ループの終了・待機まわりを DB / netkeiba 無しで検証する。
    /// `max_sweeps` 回スイープしたら slots を空にして正常終了させる（無限ループにしない）。
    struct FakeSweeper {
        date: chrono::NaiveDate,
        interval_minutes: u64,
        max_sweeps: usize,
        sweeps: AtomicUsize,
        /// スイープ時刻を標準出力に出すか（手動のスリープ検証で目視するため）。
        verbose: bool,
    }

    impl Sweeper for FakeSweeper {
        type Slot = ();

        async fn load_slots(&self) -> anyhow::Result<Vec<()>> {
            if self.sweeps.load(Ordering::SeqCst) >= self.max_sweeps {
                // 対象開催なし扱い＝ should_continue が false になり正常終了する。
                return Ok(vec![]);
            }
            Ok(vec![()])
        }

        /// 常に「これから発走」に見える post_time（＝ classify は Due を返し続ける）。
        fn post_time(_slot: &()) -> Option<NaiveTime> {
            Some(NaiveTime::from_hms_opt(23, 59, 0).unwrap())
        }

        fn has_result(_slot: &()) -> bool {
            false
        }

        /// windowless（odds-collect 相当）＝発走前なら常に Due。
        fn window(&self) -> Option<Duration> {
            None
        }

        fn date(&self) -> chrono::NaiveDate {
            self.date
        }

        fn once(&self) -> bool {
            false
        }

        fn interval_minutes(&self) -> u64 {
            self.interval_minutes
        }

        async fn sweep(&mut self, slots: &[()], _statuses: &[RaceStatus], _now: NaiveTime) {
            // 終了判定の直前には slots 空のスイープが 1 回走る（ループ骨格の仕様）。実スイープだけを
            // 数えたいので空巡は勘定に入れない。
            if slots.is_empty() {
                return;
            }
            let n = self.sweeps.fetch_add(1, Ordering::SeqCst) + 1;
            if self.verbose {
                println!("[{}] スイープ #{n}", Local::now().format("%H:%M:%S"));
            }
        }

        fn finish_noun(&self) -> &str {
            "監視"
        }
    }

    #[tokio::test]
    async fn stops_after_one_sweep_when_target_date_has_passed() {
        // 対象日が過去 ＝ 日付を跨いだ状態。Due が残り should_continue は true のままなので、
        // 日付判定が無ければ永久ループになる（#568 の実害: 最終レースから 14 時間経っても終了せず）。
        let mut sweeper = FakeSweeper {
            date: (Local::now() - Duration::days(1)).date_naive(),
            // 日付判定で抜けるので待機には入らない。入ってしまったらテストが固まって気づける。
            interval_minutes: 60,
            max_sweeps: usize::MAX,
            sweeps: AtomicUsize::new(0),
            verbose: false,
        };
        run_monitor_loop(&mut sweeper).await.unwrap();
        // 判定はスイープ後なので、終了する前に必ず 1 スイープは走る（過去日でも無言 no-op にしない）。
        assert_eq!(sweeper.sweeps.load(Ordering::SeqCst), 1);
    }

    /// 実機のスリープ跨ぎを目視で確認する手動テスト（#568）。DB / netkeiba を一切使わず、
    /// 本番と同じ [`run_monitor_loop`] を 1 分間隔で回す。
    ///
    /// ```sh
    /// cargo test -p monitor-loop -- --ignored --nocapture crosses_real_host_sleep
    /// # 起動後に別端末で `pmset sleepnow`（caffeinate -i は強制スリープを止めないので寝る）。
    /// # 復帰させると「⚠ スイープが N 分途切れました」＋直後のスイープが出れば OK。
    /// ```
    ///
    /// 自動テストにしないのは、実スリープが人手（復帰操作）と数分の実時間を要するため。
    /// CI では `--ignored` によりスキップされるが、コンパイルは通るので腐らない。
    #[tokio::test]
    #[ignore = "実機スリープを跨ぐ目視確認用（数分＋手動の復帰操作が要る）"]
    async fn crosses_real_host_sleep() {
        let mut sweeper = FakeSweeper {
            date: Local::now().date_naive(),
            interval_minutes: 1,
            max_sweeps: 5,
            sweeps: AtomicUsize::new(0),
            verbose: true,
        };
        run_monitor_loop(&mut sweeper).await.unwrap();
        assert_eq!(sweeper.sweeps.load(Ordering::SeqCst), 5);
    }
}
