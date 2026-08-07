use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime};

use crate::{
    RaceStatus, capped_wait, classify, count_started_before_post, detect_sweep_gap,
    effective_interval_minutes, should_continue, should_stop_by_date,
};

/// 現在時刻を返す関数。実運用は `Local::now`、テストは固定/前進する偽時計を渡す。
/// ループの時刻依存を 1 箇所に集約し、日付跨ぎのような時間依存の挙動を実時間なしで検証できるようにする。
type Clock = fn() -> DateTime<Local>;

/// wall-clock 期限を待つときの刻み幅。単発の長い sleep はホストのスリープを跨げないため、この
/// 間隔で現在時刻と期限を比べ直す（#568）。
///
/// **5 秒に置く根拠は 8/1 の実事象**（`docs/original-docs/568-monitor-sleep-gap.md`）。`tokio::time::sleep`
/// は単調時計基準で、ホストのスリープ中は進まない＝ティックは「起きている時間」でしか消化されない。
/// あの日の Standby は **7 秒の DarkWake が 4 回（累計 28 秒）** だったので、30 秒刻みでは 1 ティックも
/// 満了せず再スイープに到達しない。5 秒なら DarkWake 1 回ごとに満了し、期限超過を検知できる。
/// 空回りのコストは最短間隔 1 分で 12 回 / 既定なら predict-watch 5 分＝60 回・odds-collect 15 分＝180 回。
/// 1 回あたりは時刻比較のみなので無視できる。
const WAKE_CHECK_TICK: StdDuration = StdDuration::from_secs(5);

/// wall-clock の期限まで `tick` 刻みで待つ（#568）。
///
/// `tokio::time::sleep(長時間)` 1 回だとホストがスリープした際にタイマーが進まず、復帰後も
/// 「残り」を待ち続けて監視が無言で止まる。毎ティック現在時刻と期限を比べ直すことで、復帰時点で
/// 期限を過ぎていれば即座に抜ける＝次スイープが走る。
///
/// `tick` は実運用では `WAKE_CHECK_TICK` 固定。テストが実時間を待たずに「複数ティックを跨ぐ」
/// 経路を踏めるよう引数にしている。
async fn sleep_until_with_tick(deadline: DateTime<Local>, tick: StdDuration, now: Clock) {
    // 期限を過ぎている（差が負）なら to_std が Err になり、ループを抜ける＝即座に次スイープへ。
    while let Ok(remaining) = (deadline - now()).to_std() {
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(remaining.min(tick)).await;
    }
}

/// 次スイープの期限。`capped_wait` を通すのは `DateTime + Duration` が範囲外で panic するため
/// （巨大な `--interval` を渡されても異常終了させない）。
fn next_deadline(now: DateTime<Local>, interval_minutes: u64) -> DateTime<Local> {
    now + capped_wait(interval_minutes)
}

/// 次スイープの時刻まで wall-clock で待つ（#568）。
///
/// 途切れの判定はここではなく呼び出し側（スイープ開始時刻どうしの間隔）で行う。待機区間だけを
/// 測るとスイープ実行中に寝られたケースを取りこぼすため（`sweeper.sweep` は predict-watch では
/// scrape_delay × 対象レースぶん数分かかる）。
async fn wait_until_next_sweep(interval_minutes: u64, now: Clock) {
    sleep_until_with_tick(next_deadline(now(), interval_minutes), WAKE_CHECK_TICK, now).await;
}

/// 前スイープからの空きが想定を超えていたら出す警告文（純関数・単体テスト対象）。
///
/// **継続時にも終了時にも通す**のが肝。日付を跨いで復帰したケース（#568 の実事象そのもの）は
/// 終了経路に入るので、そこで黙ると「丸一日未監視だった日」が警告なしの正常終了に見える。
fn sweep_gap_notice(
    interval_minutes: u64,
    since_last_sweep_start: Duration,
    last_cycle_took: Duration,
) -> Option<String> {
    detect_sweep_gap(interval_minutes, since_last_sweep_start, last_cycle_took).map(|minutes| {
        format!(
            "⚠ 前回スイープから {minutes} 分空きました（想定 {interval_minutes} 分間隔）。ホストのスリープ/停止、\
             またはレース一覧の取得失敗が続いた可能性があります。空いていた間に発走したレースは評価されていません。"
        )
    })
}

/// 日付跨ぎで終了するときに出す行を組み立てる純関数（#568・単体テスト対象）。
///
/// **途切れ警告をここに含めるのが要点**。日付を跨いで復帰したケース（#568 の実事象そのもの）は
/// この終了経路に入るので、警告を継続経路にしか置かないと「丸一日未監視だった日」が
/// 警告なしの正常終了に見える。`last_sweep` は（前スイープ開始時刻, その所要時間）。
fn date_stop_lines(
    noun: &str,
    date: NaiveDate,
    interval_minutes: u64,
    last_sweep: Option<(DateTime<Local>, Duration)>,
    entered_target_date: bool,
    now: DateTime<Local>,
) -> Vec<String> {
    let mut lines = Vec::new();
    match last_sweep {
        Some((prev, took)) => {
            if let Some(notice) = sweep_gap_notice(interval_minutes, now - prev, took) {
                lines.push(notice);
            }
            lines.push(format!("   （最終スイープ: {}）", prev.format("%m-%d %H:%M")));
        }
        // 対象日を回っていたのにスイープ 0 回＝丸一日レースを評価できていない。ここを黙ると
        // 終了行だけになり、正常な後始末と見分けがつかない（#568 と同型の静かな失敗）。
        // 起動時点で既に過去日だった場合（--date の打ち間違い等）は対象外——この強い警告を出すと
        // 単なるタイポが DB 障害と同じ見え方になる。誤用は warn_if_not_today_jst が別途伝える。
        None if entered_target_date => lines.push(
            "⚠ 対象日中に一度もスイープできませんでした（レース一覧の取得が続けて失敗した可能性）。\
             この日のレースは 1 つも評価されていません。"
                .to_string(),
        ),
        None => {}
    }
    lines.push(format!(
        "── {noun}終了: 対象日（{date}）を過ぎました。発走状態は時刻のみで判定するため、日付を跨いだら終了します。"
    ));
    lines
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
/// ホストのスリープを跨いでも監視が死なないよう、待機は wall-clock 期限で刻み（`sleep_until_with_tick`）、
/// スイープ間隔の途切れを検知して警告し、日付を跨いだら終了する（`should_stop_by_date`・#568）。
pub async fn run_monitor_loop<S: Sweeper>(sweeper: &mut S) -> anyhow::Result<()> {
    run_monitor_loop_with(sweeper, Local::now).await
}

async fn run_monitor_loop_with<S: Sweeper>(sweeper: &mut S, now: Clock) -> anyhow::Result<()> {
    let date = sweeper.date();
    let once = sweeper.once();
    // 待機と途切れ判定で同じ値を使う（片方だけ丸めると閾値が実間隔に追随せず検知が死ぬ）。
    let requested_interval = sweeper.interval_minutes();
    let interval = effective_interval_minutes(requested_interval);
    if interval != requested_interval {
        let reason = if requested_interval < interval {
            "下限を下回る"
        } else {
            "上限を超える"
        };
        println!(
            "⚠ --interval {requested_interval} 分は{reason}ため {interval} 分として扱います。"
        );
    }
    let window = sweeper.window();
    let noun = sweeper.finish_noun().to_string();
    let fetch_card_hint = sweeper.fetch_card_hint().to_string();

    // 直前スイープの開始時刻と所要時間。スイープ開始どうしの間隔で途切れを測るため保持する
    //（待機区間だけを測るとスイープ実行中に寝られたケースを取りこぼす）。所要時間は閾値に足して、
    // スイープが長い日に毎サイクル誤警告が出るのを防ぐ。
    let mut last_sweep_started: Option<DateTime<Local>> = None;
    let mut last_cycle_took = Duration::zero();
    // 起動直後に過去日と分かったケース（--date の打ち間違い等）と、当日回していてスイープ 0 回のまま
    // 日付を跨いだケースを区別するためのフラグ。前者に「一度もスイープできませんでした」を出すと
    // 単なるタイポが DB 障害と同じ見え方になる。
    let mut entered_target_date = false;

    loop {
        // 日付跨ぎの終了判定は「ループ先頭」に置く（#568）。発走状態判定（classify）は時刻だけを
        // 見るため、日付を跨ぐと昨日のレースが再び「発走前」に見えて should_continue が永久に
        // true を返す。ここに置くことで (1) 前日レースを再スクレイプしてオッズ時系列を汚す 1 巡が
        // 走らない、(2) load_slots が失敗し続ける経路（下の continue）でも必ず判定を通る。
        // 日付はホストのローカル日付（運用ホストは JST 固定。前提は warn_if_not_today_jst が点検）。
        let cycle_started = now();
        if should_stop_by_date(date, cycle_started.date_naive()) {
            // --once は「明示的に 1 スイープだけ回す」指定（cron / 検証用）なので止めはしないが、
            // 過去日を渡していることは伝える（黙って発走済みレースを再取得しない）。
            if once {
                println!(
                    "⚠ --once で対象日（{date}）を過ぎた日付を指定しています。発走済みレースのオッズを\
                     再取得して保存する点に注意してください。"
                );
            } else {
                // 日付跨ぎ復帰は #568 の実事象そのもの。終了行だけを出すと「丸一日未監視だった日」が
                // 警告なしの正常終了に見えるため、空き時間と最終スイープ時刻を必ず添える。
                for line in date_stop_lines(
                    &noun,
                    date,
                    interval,
                    last_sweep_started.map(|prev| (prev, last_cycle_took)),
                    entered_target_date,
                    cycle_started,
                ) {
                    println!("{line}");
                }
                break;
            }
        }

        entered_target_date = true;

        // サイクル所要は**単調時計**で測る。ホストのスリープ中は進まないので、スリープぶんが
        // 「スイープ所要」として閾値に吸われない（吸われると sleep 中の沈黙を検知できなくなる）。
        let cycle_clock = Instant::now();

        // 前スイープからの間隔が想定を大きく超えていたら、その間 監視が止まっていた事実を必ず出す。
        // **load_slots より前**に置く: DB 障害が続くサイクルでも空きを報告するため（ホスト復帰直後に
        // Postgres がまだ上がっていない状況は常態で、そこで黙ると「何時間飛んだか」を知る術がない）。
        // last_sweep_started の更新はスイープが実際に走ったときだけなので、復旧までは毎巡
        // 「累積した空き」を報告し続ける。
        if let Some(prev) = last_sweep_started
            && let Some(notice) = sweep_gap_notice(interval, cycle_started - prev, last_cycle_took)
        {
            println!("{notice}");
        }

        // 継続監視中の一時的 DB エラーでプロセスを落とすと取りこぼす。握って次スイープへ続行する
        //（--once 時のみ伝播して非ゼロ終了）。
        let slots = match sweeper.load_slots().await {
            Ok(s) => s,
            Err(e) if once => return Err(e),
            Err(e) => {
                println!("⚠ レース一覧の取得に失敗（次スイープで再試行）: {e}");
                wait_until_next_sweep(interval, now).await;
                continue;
            }
        };
        last_sweep_started = Some(cycle_started);

        // 発走状態判定に渡す時刻（Clock の `now` と紛れないよう別名にする）。
        let now_time = cycle_started.time();
        // 発走状態は 1 スイープ 1 回だけ算出し、sweep 表示と終了判定で共有する。
        let statuses: Vec<RaceStatus> = slots
            .iter()
            .map(|s| classify(now_time, S::post_time(s), S::has_result(s), window))
            .collect();

        // 防御: 発走前（now <= post）なのに結果取込済みのレースは、races_by_date の不変条件
        //（発走前＝race_cards 由来で track_condition=NULL）が崩れた兆候。放置すると Started 誤判定で
        // 監視が無言 no-op 化するため、検出したら警告する（#459 で両 app に共通化）。
        let started_before_post =
            count_started_before_post(&slots, now_time, S::post_time, S::has_result);
        if started_before_post > 0 {
            println!(
                "⚠ 発走前なのに結果取込済みのレースが {started_before_post} 件あります。発走状態判定の前提が \
                 崩れている可能性があり、対象から外れます（fetch-card / 成績取込の状態を確認してください）。"
            );
        }

        sweeper.sweep(&slots, &statuses, now_time).await;
        // 次巡の途切れ判定に使う。長いサイクル（DB ロード＋スイープ）を「途切れ」と誤検知しないよう
        // 閾値へ足すが、単調時計で測るのでホストのスリープぶんは含まれない。
        last_cycle_took =
            Duration::from_std(cycle_clock.elapsed()).unwrap_or_else(|_| Duration::zero());

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
        wait_until_next_sweep(interval, now).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::TimeZone;

    use super::*;

    // --- sleep_until_with_tick: wall-clock 期限で刻んで待つ（#568） ---

    #[tokio::test]
    async fn returns_immediately_when_deadline_already_passed() {
        // スリープから復帰した直後の状況（期限をとうに過ぎている）。待たずに抜けて次スイープへ進む。
        let started = Local::now();
        sleep_until_with_tick(
            started - Duration::hours(4),
            StdDuration::from_millis(50),
            Local::now,
        )
        .await;
        assert!((Local::now() - started) < Duration::seconds(1));
    }

    #[tokio::test]
    async fn waits_until_deadline_across_multiple_ticks() {
        // 期限までティックを複数回跨いで待つ（実運用の 5 秒刻みを 20ms に縮めた等価経路）。
        let started = Local::now();
        let deadline = started + Duration::milliseconds(200);
        sleep_until_with_tick(deadline, StdDuration::from_millis(20), Local::now).await;
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
        /// `load_slots` を常に失敗させる（DB 障害の再現）。
        fail_load: bool,
        /// `load_slots` が呼ばれた回数（失敗経路が回り続けていないかの観測用）。
        loads: AtomicUsize,
        /// スイープ時刻を標準出力に出すか（手動のスリープ検証で目視するため）。
        verbose: bool,
        /// post_time の算出に使う時計（ループへ渡すものと揃える）。
        now: Clock,
        /// `--once` 相当（単発実行）か。
        once: bool,
    }

    impl FakeSweeper {
        /// 発走前レースが残り続ける（＝時刻軸では終われない）既定の fake。
        fn new(date: chrono::NaiveDate) -> Self {
            Self {
                date,
                now: Local::now,
                once: false,
                // 日付判定で抜けるので待機には入らない。入ってしまったらテストが遅延して気づける。
                interval_minutes: 60,
                max_sweeps: usize::MAX,
                sweeps: AtomicUsize::new(0),
                fail_load: false,
                loads: AtomicUsize::new(0),
                verbose: false,
            }
        }
    }

    impl Sweeper for FakeSweeper {
        /// Slot 自身が post_time を持つ。固定時刻（23:59 等）にすると実行時刻しだいで
        /// 「発走済み」に転んでテストが日付境界で落ちるため、常に現在時刻から算出する。
        type Slot = NaiveTime;

        async fn load_slots(&self) -> anyhow::Result<Vec<NaiveTime>> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            if self.fail_load {
                anyhow::bail!("fake DB error");
            }
            if self.sweeps.load(Ordering::SeqCst) >= self.max_sweeps {
                // 対象開催なし扱い＝ should_continue が false になり正常終了する。
                return Ok(vec![]);
            }
            // 「これから発走」に見える post_time（＝ classify は Due を返し続ける）。
            // 1 分後が翌日へ回り込む深夜帯は 23:59:59 に丸め、同日内で必ず now より後に保つ。
            let (post, overflowed) = (self.now)()
                .time()
                .overflowing_add_signed(Duration::minutes(1));
            Ok(vec![if overflowed == 0 {
                post
            } else {
                NaiveTime::from_hms_opt(23, 59, 59).expect("固定値")
            }])
        }

        fn post_time(slot: &NaiveTime) -> Option<NaiveTime> {
            Some(*slot)
        }

        fn has_result(_slot: &NaiveTime) -> bool {
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
            self.once
        }

        fn interval_minutes(&self) -> u64 {
            self.interval_minutes
        }

        async fn sweep(&mut self, slots: &[NaiveTime], _statuses: &[RaceStatus], _now: NaiveTime) {
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

    /// 日付判定が退行するとループが終わらずテストが固まるので、必ず時間上限で包む。
    /// 退行時に「無関係なテストごと CI のジョブ timeout で落ちる」のを避ける。
    async fn run_bounded(sweeper: &mut FakeSweeper) {
        tokio::time::timeout(
            StdDuration::from_secs(10),
            run_monitor_loop_with(sweeper, Local::now),
        )
        .await
        .expect("run_monitor_loop が終了しない（日付跨ぎの終了判定が効いていない）")
        .unwrap();
    }

    // --- sweep_gap_notice: 途切れ警告の組み立て（#568） ---

    #[test]
    fn no_notice_when_sweeps_are_on_schedule() {
        assert!(sweep_gap_notice(5, Duration::minutes(6), Duration::minutes(1)).is_none());
    }

    #[test]
    fn notice_reports_the_silent_span() {
        // #568 の実事象相当（想定 5 分・前スイープから 234 分）。
        let notice = sweep_gap_notice(5, Duration::minutes(234), Duration::zero())
            .expect("途切れとして検知されるべき");
        assert!(notice.contains("234 分空きました"), "{notice}");
        assert!(notice.contains("評価されていません"), "{notice}");
    }

    #[test]
    fn long_sweep_does_not_trigger_false_notice() {
        // スイープ所要が interval を超える日（predict-watch の scrape_delay × 対象レース）。
        // 閾値に前スイープ所要を足しているので、想定内の遅れでは警告しない。
        assert!(sweep_gap_notice(5, Duration::minutes(18), Duration::minutes(9)).is_none());
        // 所要を足しても越える空きは検知する。
        assert!(sweep_gap_notice(5, Duration::minutes(40), Duration::minutes(9)).is_some());
    }

    #[test]
    fn absurd_interval_does_not_panic_on_deadline() {
        // `DateTime + Duration` は範囲外で panic する。巨大 interval でも期限計算が壊れないこと。
        let now = Local::now();
        let deadline = next_deadline(now, u64::MAX);
        assert_eq!(deadline - now, Duration::days(1));
    }

    #[tokio::test]
    async fn stops_without_sweeping_when_target_date_has_passed() {
        // 対象日が過去 ＝ 日付を跨いだ状態。Due が残り should_continue は true のままなので、
        // 日付判定が無ければ永久ループになる（#568 の実害: 最終レースから 14 時間経っても終了せず）。
        let mut sweeper = FakeSweeper::new((Local::now() - Duration::days(1)).date_naive());
        run_bounded(&mut sweeper).await;
        // 判定はループ先頭なので 1 スイープも走らない。前日レースを再スクレイプして
        // オッズ時系列を汚す 1 巡を作らないため（起動時の warn_if_not_today_jst が誤用は伝える）。
        assert_eq!(sweeper.sweeps.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn date_check_precedes_load_slots_so_db_errors_cannot_bypass_it() {
        // 日付判定が load_slots より前にあること（＝ DB エラーで continue し続ける経路が
        // 判定を迂回できないこと）を、失敗する load_slots に一度も到達しない事実で示す。
        // 判定がループ末尾にあった 1 巡目は、この経路だけ迂回されて「終われない」状態が残っていた。
        let mut sweeper = FakeSweeper {
            fail_load: true,
            ..FakeSweeper::new((Local::now() - Duration::days(1)).date_naive())
        };
        run_bounded(&mut sweeper).await;
        // 日付判定が load_slots より前にあるので、失敗経路にすら入らずに終了する。
        assert_eq!(sweeper.loads.load(Ordering::SeqCst), 0);
    }

    // --- date_stop_lines: 日付跨ぎ終了時の出力（2 巡目の [Must-fix] の回帰テスト・#568） ---

    #[test]
    fn date_stop_output_carries_the_gap_warning() {
        // #568 の実事象: 前日 14:32 のスイープを最後に寝て、翌朝復帰 → 日付跨ぎで終了する経路。
        // ここで終了行しか出さないと「丸一日未監視だった日」が正常終了に見える（実際そうなっていた）。
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let prev = at_local(2026, 8, 1, 14, 32);
        let now = at_local(2026, 8, 2, 8, 55);
        let lines = date_stop_lines("監視", date, 5, Some((prev, Duration::zero())), true, now);
        assert!(
            lines[0].contains("分空きました"),
            "終了時にも途切れ警告を出すこと: {lines:?}"
        );
        assert!(lines[1].contains("08-01 14:32"), "{lines:?}");
        assert!(lines[2].contains("対象日"), "{lines:?}");
    }

    #[test]
    fn date_stop_output_is_quiet_when_sweeps_were_on_schedule() {
        // 最終レース後もプロセスが残って日付を跨いだだけなら、余計な警告は出さない。
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let prev = at_local(2026, 8, 1, 23, 58);
        let now = at_local(2026, 8, 2, 0, 1);
        let lines = date_stop_lines("監視", date, 5, Some((prev, Duration::zero())), true, now);
        assert!(!lines[0].contains("分空きました"), "{lines:?}");
    }

    #[test]
    fn date_stop_output_flags_a_day_without_any_sweep() {
        // 対象日中に一度もスイープできないまま日付を跨いだ日（DB 障害が続いた等）。
        // 終了行だけだと「正常な後始末」と見分けがつかないので、必ず警告を添える。
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let now = at_local(2026, 8, 2, 0, 1);
        let lines = date_stop_lines("収集", date, 5, None, true, now);
        assert!(
            lines[0].contains("一度もスイープできませんでした"),
            "{lines:?}"
        );
        assert_eq!(lines.len(), 2, "{lines:?}");
    }

    #[test]
    fn date_stop_output_is_quiet_when_the_date_was_already_past_at_startup() {
        // --date の打ち間違い等で起動時点から過去日だったケース。強い警告を出すと、単なるタイポが
        // 「DB 障害で丸一日飛んだ」と同じ見え方になる。誤用は warn_if_not_today_jst が別途伝える。
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let lines = date_stop_lines("監視", date, 5, None, false, at_local(2026, 8, 5, 12, 0));
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("対象日"), "{lines:?}");
    }

    thread_local! {
        /// 読むたびに 1 時間進む偽時計の現在値（epoch 秒）。**thread_local** にしてあるので、
        /// この時計を使うテストが増えても並列実行で相互汚染しない（`#[tokio::test]` は
        /// current_thread ランタイムなので、ループ内の await も同じスレッドで走る）。
        static FAKE_EPOCH: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }

    fn set_fake_clock(at: DateTime<Local>) {
        FAKE_EPOCH.with(|c| c.set(at.timestamp()));
    }

    fn fake_clock() -> DateTime<Local> {
        let secs = FAKE_EPOCH.with(|c| {
            let v = c.get();
            c.set(v + 3600);
            v
        });
        Local.timestamp_opt(secs, 0).single().expect("有効な epoch")
    }

    /// ホストのローカル時刻で固定時点を作る。本体もローカル日付で判定するので、これで
    /// どのタイムゾーンでも同じ相対関係（例: 20:00 から 4 時間で日付を跨ぐ）になる。
    fn at_local(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, m, d, hh, mm, 0)
            .single()
            .expect("DST の谷間でない固定値")
    }

    #[tokio::test]
    async fn once_sweeps_a_past_date_with_a_warning() {
        // --once は日付では止めない（cron / 検証用の明示指定）。警告を出したうえで 1 スイープ走る。
        // ADR 0072 が「--once はこの保護の対象外」と決めた挙動。
        let mut sweeper = FakeSweeper {
            once: true,
            ..FakeSweeper::new((Local::now() - Duration::days(1)).date_naive())
        };
        tokio::time::timeout(
            StdDuration::from_secs(10),
            run_monitor_loop_with(&mut sweeper, Local::now),
        )
        .await
        .expect("--once は 1 スイープで終了すること")
        .unwrap();
        assert_eq!(sweeper.sweeps.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rolls_over_to_the_stop_path_after_sweeping() {
        // 「当日 1 スイープ → 時間が飛ぶ → 日付跨ぎで終了」をループ全体で踏む。実時間は待たない
        //（偽時計が期限を追い越すので待機は即座に抜ける）。
        // 起点はローカル 20:00。本体もローカル日付で判定するので、どの TZ でも 4 時間後に日付を跨ぐ。
        let base = at_local(2026, 8, 1, 20, 0);
        set_fake_clock(base);
        let mut sweeper = FakeSweeper {
            interval_minutes: 60,
            now: fake_clock,
            ..FakeSweeper::new(base.date_naive())
        };
        tokio::time::timeout(
            StdDuration::from_secs(10),
            run_monitor_loop_with(&mut sweeper, fake_clock),
        )
        .await
        .expect("日付跨ぎで終了すること")
        .unwrap();
        // 当日ぶんは 1 スイープ走り、日付が変わった時点で終了している。
        assert_eq!(sweeper.sweeps.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keeps_running_while_still_on_target_date() {
        // 当日は日付判定で止まらないこと（判定をループ先頭へ移した際に「当日でも即終了」させて
        // しまう退行を検出する）。偽時計を朝に固定し、対象日を跨がない範囲でスイープを重ねる。
        // 実時計だと JST 00:00 跨ぎや CI 負荷で結果が揺れるため使わない。
        let base = at_local(2026, 8, 1, 6, 0);
        set_fake_clock(base);
        let mut sweeper = FakeSweeper {
            interval_minutes: 60,
            now: fake_clock,
            // 3 スイープしたら slots を空にして正常終了させる（当日中に終わる経路）。
            max_sweeps: 3,
            ..FakeSweeper::new(base.date_naive())
        };
        tokio::time::timeout(
            StdDuration::from_secs(10),
            run_monitor_loop_with(&mut sweeper, fake_clock),
        )
        .await
        .expect("当日中に正常終了すること")
        .unwrap();
        // 日付判定で即終了していれば 0 回。当日として回れば 3 回走る。
        assert_eq!(sweeper.sweeps.load(Ordering::SeqCst), 3);
    }

    /// 実機のスリープ跨ぎを目視で確認する手動テスト（#568）。DB / netkeiba を一切使わず、
    /// 本番と同じ [`run_monitor_loop`] を 1 分間隔で回す。
    ///
    /// ```sh
    /// cargo test -p monitor-loop -- --ignored --nocapture crosses_real_host_sleep
    /// # 起動後に別端末で `pmset sleepnow`。
    /// # 復帰させると「⚠ 前回スイープから N 分空きました」＋直後のスイープが出れば OK。
    /// ```
    ///
    /// 自動テストにしないのは、実スリープが人手（復帰操作）と数分の実時間を要するため。
    /// CI では `--ignored` によりスキップされるが、コンパイルは通るので腐らない。
    /// ここだけは公開入口（[`run_monitor_loop`]）を呼ぶ。
    #[tokio::test]
    #[ignore = "実機スリープを跨ぐ目視確認用（数分＋手動の復帰操作が要る）"]
    async fn crosses_real_host_sleep() {
        let mut sweeper = FakeSweeper {
            interval_minutes: 1,
            max_sweeps: 5,
            verbose: true,
            ..FakeSweeper::new(Local::now().date_naive())
        };
        run_monitor_loop(&mut sweeper).await.unwrap();
        assert_eq!(sweeper.sweeps.load(Ordering::SeqCst), 5);
    }
}
