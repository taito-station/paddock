//! `--overview`（[`predict::session::run_overview`], #551）が予想セッション状態に
//! 干渉しないことを Postgres（`#[sqlx::test]` の一時DB）往復で固定する（#555）。
//!
//! 対象は `predict_sessions` / `predict_bets` / `predict_race_conditions` /
//! `predict_race_skips` の 4 テーブル。オッズ系（`race_odds`）は read-through の
//! 副作用として書かれうるのが仕様どおりなので対象外（下記カナリアは別目的）。
//!
//! ## ネットワークに出ない設計（変更するときは必ず読むこと）
//!
//! seed するのは `races` のみで、`race_cards` は **絶対に入れない**。
//! `find_races_by_date` は `races`（`source='pdf'` に限る）∪ `race_cards` なので
//! レースは列挙されるが、`render_race_prediction` が最初に呼ぶ `predict_race_views` は
//! `find_race_card`（`race_cards` のみ参照）が `None` を返すため `NotFound` になり、
//! `RaceView::NoEntries` で即 return する。オッズ read-through
//! （`app.odds.race_odds` → `UreqNetkeibaScraper`）はその return より後ろなので到達しない。
//! `race_cards` を seed すると CI が netkeiba へ実通信するテストになる。
//!
//! この前提は [`assert_no_scrape_side_effects`] が `race_cards` 0 件（不変条件そのもの）と
//! `race_odds` 0 件（read-through 到達のカナリア）の両方で機械的に見張る。
//!
//! ## テストが空回りしないための前提 assert
//!
//! `races` の列挙は `source='pdf'` に依存しており、`save_race` はこの列を bind せず
//! 列 DEFAULT に頼っている。既定が変わるとレースが 0 件になり、非干渉 assert は
//! 「何も起きていないので一致」で緑のまま空回りする。それを防ぐため各テストは
//! `run_overview` の前に「対象日が N レース列挙される」ことを assert する。
//!
//! ## このテストがカバーしないこと
//!
//! `RaceView::NoOdds` / `Shown` 経路（出馬表とオッズが揃ったレースの EV 表示）。
//! そこへ後から書き込みが足された場合は検知できない。EV 表示経路を張るには
//! `App` のスクレイパ（`UreqNetkeibaScraper` 具象固定）をジェネリック化して
//! フェイクを注入する必要があり、本 issue のスコープ外。EV 表示経路そのものの回帰は、
//! 全 repository を mock した `src/use-case/tests/test_predict_race.rs` が
//! DB・ネットワーク非依存で担う。

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use netkeiba_scraper::UreqNetkeibaScraper;
use paddock_domain::{Race, RaceId, Surface, TrackCondition, Venue};
use paddock_use_case::repository::{
    PredictBetRecord, PredictRaceConditionRecord, PredictSessionRecord, PredictSessionRepository,
    RaceRepository,
};
use paddock_use_case::{Interactor, NoopFetcher, NoopParser, OddsInteractor, SettleInteractor};
use predict::session::run_overview;
use predict::setup::App;
use rdb_gateway::PostgresRepository;
use sqlx::PgPool;

/// 馬場入力を「値あり」で記録済みのレース。
const RACE_RECORDED: &str = "2026-3-nakayama-8-1R";
/// 馬場入力を「不明として記録済み」のレース。
const RACE_RECORDED_UNKNOWN: &str = "2026-3-nakayama-8-2R";
/// 馬場入力が **未記録** のレース。[`seed_recorded_session`] は意図的にこれを記録しない。
///
/// `run_race` の馬場保存は無条件ではなく `recorded != Some(解決値)` の条件付きで走る（#80）。
/// 記録済みレースだけを seed すると、その条件付き書き込みが `run_overview` に紛れ込んでも
/// 条件が成立せず 1 行も書かれないため検知できない。未記録レースを 1 本混ぜることで
/// 「記録済みと同値なら書かない」実装をコピペしても必ず発火する状態を作る。
const RACE_UNRECORDED: &str = "2026-3-nakayama-8-3R";
/// 別日（[`other_date`]）のレース。session 対象日の race_id を使い回すと
/// 「同じ race_id が 2 つの日付を指す」状態になって読みにくいので分ける。
const RACE_OTHER_DAY: &str = "2026-3-nakayama-9-1R";

/// 予想セッションを張る対象日。
fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 19).unwrap()
}

/// `races` は入っているがセッション対象日ではない日（「開催なし」テストで DB を空にしないため）。
fn other_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 26).unwrap()
}

/// 固定時刻。`predict_*` の時刻列は TEXT（rfc3339）なのでスナップショット比較を読みやすくする。
fn fixed_at(sec: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 19, 9, 0, sec).unwrap()
}

fn race_id(value: &str) -> RaceId {
    RaceId::try_from(value).unwrap()
}

/// 成績(`races`)行。**`race_cards` は作らない**（冒頭の「ネットワークに出ない設計」参照）。
fn race(on: NaiveDate, id: &str, day: u32, race_num: u32) -> Race {
    Race {
        race_id: race_id(id),
        date: on,
        venue: Venue::Nakayama,
        round: 3,
        day,
        race_num,
        surface: Surface::Turf,
        distance: 2000,
        // 未記録レースのフォールバック元になる確定値。
        track_condition: Some(TrackCondition::Firm),
        weather: None,
        results: Vec::new(),
    }
}

/// テスト用の [`App`]。`build_app` は env（`PADDOCK_DB_URL`）と実 DB を読むので使わず、
/// `#[sqlx::test]` が用意した一時DBのプールを直接挿す。
///
/// スクレイパは本番と同じ具象型（`App` のフィールドが具象型固定でダブルに差し替えられない）だが、
/// 本テストは NoEntries 経路しか通らないため呼ばれない。
fn test_app(pool: &PgPool) -> App {
    App {
        interactor: Interactor::new(
            PostgresRepository::new(pool.clone()),
            NoopParser,
            NoopFetcher,
        ),
        odds: OddsInteractor::new(
            UreqNetkeibaScraper::new(),
            PostgresRepository::new(pool.clone()),
        ),
        settle: SettleInteractor::new(
            UreqNetkeibaScraper::new(),
            PostgresRepository::new(pool.clone()),
        ),
    }
}

/// 対象日の `races` を 3 本 seed する（記録済み 2 本 + 未記録 1 本）。
async fn seed_races_on_date(repo: &PostgresRepository) {
    repo.save_race(&race(date(), RACE_RECORDED, 8, 1))
        .await
        .unwrap();
    repo.save_race(&race(date(), RACE_RECORDED_UNKNOWN, 8, 2))
        .await
        .unwrap();
    repo.save_race(&race(date(), RACE_UNRECORDED, 8, 3))
        .await
        .unwrap();
}

/// 完了済みセッションを 4 テーブルすべてに実データがある状態で作る。
///
/// 厳密な `--skip-all` 相当なら `predict_bets` / `predict_race_skips` は 0 件になる
/// （`--skip-all` は `record_race_outcome` を呼ばない）。しかし空テーブルの before/after 比較は
/// 「INSERT されない」ことの検知力が弱いため、意図的に 4 テーブル全部へ行を置き
/// 「既存レコードがあっても `--overview` は一切いじらない」を固定する。
///
/// [`RACE_UNRECORDED`] の馬場入力は意図的に記録しない（定数の doc comment 参照）。
async fn seed_recorded_session(repo: &PostgresRepository) {
    // 3 つの子テーブルは predict_sessions(date) への FK を持つので、ヘッダを最初に作る。
    repo.save_predict_session(&PredictSessionRecord {
        date: date(),
        budget: 10_000,
        balance: 10_000,
        total_bet: 0,
        total_payout: 0,
        completed: false,
        created_at: fixed_at(0),
        updated_at: fixed_at(0),
    })
    .await
    .unwrap();

    // 馬場入力（値あり / 不明として記録の両方）。
    for (id, tc) in [
        (RACE_RECORDED, Some(TrackCondition::Firm)),
        (RACE_RECORDED_UNKNOWN, None::<TrackCondition>),
    ] {
        repo.save_predict_race_condition(
            date(),
            &PredictRaceConditionRecord {
                race_id: race_id(id),
                track_condition: tc,
            },
            fixed_at(1),
        )
        .await
        .unwrap();
    }

    // 買い目ありのレース → predict_bets に 1 行。
    repo.save_race_outcome(
        date(),
        &race_id(RACE_RECORDED),
        &[PredictBetRecord {
            race_id: race_id(RACE_RECORDED),
            bet_type: "win".to_string(),
            combination: "3".to_string(),
            stake: 1_000,
            payout: 0,
            ev: 1.25,
        }],
        fixed_at(2),
    )
    .await
    .unwrap();

    // 買い目なしのレース → predict_race_skips に 1 行。
    let session = repo
        .save_race_outcome(date(), &race_id(RACE_RECORDED_UNKNOWN), &[], fixed_at(3))
        .await
        .unwrap();

    repo.save_predict_session(&PredictSessionRecord {
        completed: true,
        updated_at: fixed_at(4),
        ..session
    })
    .await
    .unwrap();
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct SessionRow {
    date: String,
    budget: i64,
    balance: i64,
    total_bet: i64,
    total_payout: i64,
    completed: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct BetRow {
    bet_id: i64,
    session_date: String,
    race_id: String,
    bet_type: String,
    combination: String,
    stake: i64,
    payout: i64,
    /// `DOUBLE PRECISION` を text にキャストして取る（f64 の等値比較を避け、差分も読みやすい）。
    ev: String,
    created_at: String,
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct ConditionRow {
    session_date: String,
    race_id: String,
    track_condition: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct SkipRow {
    session_date: String,
    race_id: String,
    created_at: String,
}

/// 予想セッション 4 テーブルの全行スナップショット。時刻列は TEXT（rfc3339）なので
/// `String` のまま突き合わせる。`completed` は BOOLEAN ではなく BIGINT の 0/1。
#[derive(Debug, PartialEq)]
struct Snapshot {
    sessions: Vec<SessionRow>,
    bets: Vec<BetRow>,
    conditions: Vec<ConditionRow>,
    skips: Vec<SkipRow>,
}

async fn snapshot(pool: &PgPool) -> Snapshot {
    let sessions = sqlx::query_as::<_, SessionRow>(
        "SELECT date, budget, balance, total_bet, total_payout, completed, created_at, updated_at \
         FROM predict_sessions ORDER BY date",
    )
    .fetch_all(pool)
    .await
    .unwrap();

    let bets = sqlx::query_as::<_, BetRow>(
        "SELECT bet_id, session_date, race_id, bet_type, combination, stake, payout, \
                ev::text AS ev, created_at \
         FROM predict_bets ORDER BY bet_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();

    let conditions = sqlx::query_as::<_, ConditionRow>(
        "SELECT session_date, race_id, track_condition, created_at, updated_at \
         FROM predict_race_conditions ORDER BY session_date, race_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();

    let skips = sqlx::query_as::<_, SkipRow>(
        "SELECT session_date, race_id, created_at \
         FROM predict_race_skips ORDER BY session_date, race_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();

    Snapshot {
        sessions,
        bets,
        conditions,
        skips,
    }
}

/// 対象日のレースが期待どおり列挙されることの前提 assert。
/// これが無いと `races` が 0 件に退化したとき、非干渉 assert が空回りしたまま緑になる。
async fn assert_races_enumerated(repo: &PostgresRepository, on: NaiveDate, expected: usize) {
    let races = repo.find_races_by_date(on).await.unwrap();
    assert_eq!(
        races.len(),
        expected,
        "前提: {on} のレースが {expected} 件列挙される（0 件だと非干渉 assert が空回りする）"
    );
}

/// スクレイプ経路に到達していない（＝ネットワークに出ていない）ことの確認。
///
/// `race_cards` 0 件は冒頭の設計そのものの不変条件（seed に足した人が即座に落ちる）。
/// `race_odds` 0 件は read-through 到達のカナリア（スクレイプが失敗した場合は行が増えない
/// ので完全な検知ではないが、`render_race_prediction` の呼び出し順が壊れたことは捕まえられる）。
async fn assert_no_scrape_side_effects(pool: &PgPool) {
    let cards: i64 = sqlx::query_scalar("SELECT count(*) FROM race_cards")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        cards, 0,
        "race_cards を seed してはならない。入れると predict_race_views が成功し、\
         オッズ read-through 経由で CI が netkeiba へ実通信する（冒頭の設計注記参照）"
    );

    let odds: i64 = sqlx::query_scalar("SELECT count(*) FROM race_odds")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        odds, 0,
        "race_odds に行がある = オッズ read-through に到達しており、\
         CI が netkeiba へ実通信している（冒頭の設計注記参照）"
    );
}

/// 記録済みセッションがあるレースを `--overview` で再表示しても 4 テーブルは 1 行も変わらない。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn overview_does_not_touch_predict_session_tables(pool: PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    seed_races_on_date(&repo).await;
    seed_recorded_session(&repo).await;
    assert_races_enumerated(&repo, date(), 3).await;

    let before = snapshot(&pool).await;
    assert_eq!(before.sessions.len(), 1, "前提: セッションが 1 件ある");
    assert_eq!(before.sessions[0].completed, 1, "前提: 完了済みセッション");
    assert_eq!(before.bets.len(), 1, "前提: 買い目が 1 件ある");
    assert_eq!(
        before.conditions.len(),
        2,
        "前提: 馬場入力は 2 件のみ（3 レース目は未記録のまま）"
    );
    assert_eq!(before.skips.len(), 1, "前提: 見送りが 1 件ある");

    run_overview(&test_app(&pool), date(), 5_000, false)
        .await
        .unwrap();

    assert_eq!(
        before,
        snapshot(&pool).await,
        "--overview は予想セッション 4 テーブルを一切書き換えない（#555）"
    );
    assert_no_scrape_side_effects(&pool).await;
}

/// `--explain` を立てても非干渉。
///
/// NoEntries 経路では `explain` を消費する分岐（`conditional_gate_stats`）まで到達しないので、
/// ここで固定できるのは「`run_overview` の explain 引数の配線が書き込みを増やさない」ことだけ。
/// explain 固有ロジックそのもののカバレッジは主張しない（冒頭「カバーしないこと」参照）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn overview_with_explain_does_not_touch_predict_session_tables(pool: PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    seed_races_on_date(&repo).await;
    seed_recorded_session(&repo).await;
    assert_races_enumerated(&repo, date(), 3).await;

    let before = snapshot(&pool).await;

    run_overview(&test_app(&pool), date(), 5_000, true)
        .await
        .unwrap();

    assert_eq!(
        before,
        snapshot(&pool).await,
        "--overview --explain も予想セッション 4 テーブルを書き換えない（#555）"
    );
    assert_no_scrape_side_effects(&pool).await;
}

/// セッション未作成の日を `--overview` しても、セッションを勝手に作らない。
///
/// `--overview` は完了済みセッションの再表示だけでなく、一度も予想していない日にも撃てる。
/// find-or-create 型の実装が紛れ込むと `predict_sessions` に行ができ、
/// 「`predict_sessions` の手動 DELETE 不要」という #551 の前提が崩れる。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn overview_does_not_create_a_session_when_none_exists(pool: PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    seed_races_on_date(&repo).await;
    assert_races_enumerated(&repo, date(), 3).await;

    let before = snapshot(&pool).await;
    assert!(
        before.sessions.is_empty(),
        "前提: この日のセッションは存在しない"
    );

    run_overview(&test_app(&pool), date(), 5_000, false)
        .await
        .unwrap();

    assert_eq!(
        before,
        snapshot(&pool).await,
        "--overview はセッション未作成の日にセッションを作らない（#555）"
    );
    assert_no_scrape_side_effects(&pool).await;
}

/// 開催なし日は早期 return する（`session.rs` の `races.is_empty()` 分岐）。
///
/// `run_overview` は「この日の開催はありません」を `println!` するだけで、libtest には
/// stdout を読む安定 API が無いためメッセージ自体は assert できない。ここで固定するのは
/// 「開催なし日でも `Ok(())` で返り、セッションが存在していても何も書かない」まで。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn overview_returns_ok_and_writes_nothing_when_no_races_on_date(pool: PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    // races は別日にだけ置く（DB を空にせず、対象日だけ 0 件にする）。
    repo.save_race(&race(other_date(), RACE_OTHER_DAY, 9, 1))
        .await
        .unwrap();
    seed_recorded_session(&repo).await;
    assert_races_enumerated(&repo, date(), 0).await;
    assert_races_enumerated(&repo, other_date(), 1).await;

    let before = snapshot(&pool).await;

    run_overview(&test_app(&pool), date(), 5_000, false)
        .await
        .expect("開催なし日でもエラーにせず早期 return する");

    assert_eq!(
        before,
        snapshot(&pool).await,
        "開催なし日の早期 return でも予想セッション 4 テーブルを書き換えない（#555）"
    );
    assert_no_scrape_side_effects(&pool).await;
}
