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
//! `find_races_by_date` は races ∪ race_cards なのでレースは列挙されるが、
//! `render_race_prediction` が最初に呼ぶ `predict_race_views` は
//! `find_race_card`（`race_cards` のみ参照）が `None` を返すため `NotFound` になり、
//! `RaceView::NoEntries` で即 return する。オッズ read-through
//! （`app.odds.race_odds` → `UreqNetkeibaScraper`）はその return より後ろなので到達しない。
//! `race_cards` を seed すると CI が netkeiba へ実通信するテストになる。
//!
//! 到達していないことは各テストの `race_odds` 0 件カナリアでも見張る
//! （スクレイプが失敗した場合は行が増えないので完全な検知ではないが、
//! `render_race_prediction` の呼び出し順が壊れたことは捕まえられる）。
//!
//! ## このテストがカバーしないこと
//!
//! `RaceView::NoOdds` / `Shown` 経路（出馬表とオッズが揃ったレースの EV 表示）。
//! そこへ後から書き込みが足された場合は検知できない。EV 表示経路そのものの回帰は、
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

const RACE_1: &str = "2026-3-nakayama-8-1R";
const RACE_2: &str = "2026-3-nakayama-8-2R";

/// 予想セッションを張る対象日。
fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 19).unwrap()
}

/// `races` は入っているがセッション対象日ではない日（「開催なし」テストで DB を空にしないため）。
fn other_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 26).unwrap()
}

/// 固定時刻。`predict_*` の時刻列は TEXT（rfc3339）なのでスナップショット比較を読みやすくする。
fn t(sec: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 19, 9, 0, sec).unwrap()
}

fn race_id(value: &str) -> RaceId {
    RaceId::try_from(value).unwrap()
}

/// 成績(`races`)行。**`race_cards` は作らない**（冒頭の「ネットワークに出ない設計」参照）。
fn race(on: NaiveDate, id: &str, race_num: u32) -> Race {
    Race {
        race_id: race_id(id),
        date: on,
        venue: Venue::Nakayama,
        round: 3,
        day: 8,
        race_num,
        surface: Surface::Turf,
        distance: 2000,
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
fn test_app(pool: PgPool) -> App {
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
        settle: SettleInteractor::new(UreqNetkeibaScraper::new(), PostgresRepository::new(pool)),
    }
}

/// 完了済みセッションを 4 テーブルすべてに実データがある状態で作る。
///
/// 厳密な `--skip-all` 相当なら `predict_bets` / `predict_race_skips` は 0 件になる
/// （`--skip-all` は `record_race_outcome` を呼ばない）。しかし空テーブルの before/after 比較は
/// 「INSERT されない」ことの検知力が弱いため、意図的に 4 テーブル全部へ行を置き
/// 「既存レコードがあっても `--overview` は一切いじらない」を固定する。
async fn seed_completed_session(repo: &PostgresRepository) {
    // 3 つの子テーブルは predict_sessions(date) への FK を持つので、ヘッダを最初に作る。
    repo.save_predict_session(&PredictSessionRecord {
        date: date(),
        budget: 10_000,
        balance: 10_000,
        total_bet: 0,
        total_payout: 0,
        completed: false,
        created_at: t(0),
        updated_at: t(0),
    })
    .await
    .unwrap();

    // 馬場入力（値あり / 不明として記録の両方）。
    for (id, tc) in [
        (RACE_1, Some(TrackCondition::Firm)),
        (RACE_2, None::<TrackCondition>),
    ] {
        repo.save_predict_race_condition(
            date(),
            &PredictRaceConditionRecord {
                race_id: race_id(id),
                track_condition: tc,
            },
            t(1),
        )
        .await
        .unwrap();
    }

    // 買い目ありのレース → predict_bets に 1 行。
    repo.save_race_outcome(
        date(),
        &race_id(RACE_1),
        &[PredictBetRecord {
            race_id: race_id(RACE_1),
            bet_type: "win".to_string(),
            combination: "3".to_string(),
            stake: 1_000,
            payout: 0,
            ev: 1.25,
        }],
        t(2),
    )
    .await
    .unwrap();

    // 買い目なしのレース → predict_race_skips に 1 行。
    let session = repo
        .save_race_outcome(date(), &race_id(RACE_2), &[], t(3))
        .await
        .unwrap();

    repo.save_predict_session(&PredictSessionRecord {
        completed: true,
        updated_at: t(4),
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

/// オッズ read-through に到達していない（＝ネットワークに出ていない）ことのカナリア。
async fn assert_no_odds_written(pool: &PgPool) {
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM race_odds")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "race_odds に行がある = オッズ read-through に到達しており、\
         CI が netkeiba へ実通信している（冒頭の「ネットワークに出ない設計」参照）"
    );
}

/// 記録済みセッションがあるレースを `--overview` で再表示しても 4 テーブルは 1 行も変わらない。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn overview_does_not_touch_predict_session_tables(pool: PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race(&race(date(), RACE_1, 1)).await.unwrap();
    repo.save_race(&race(date(), RACE_2, 2)).await.unwrap();
    seed_completed_session(&repo).await;

    let before = snapshot(&pool).await;
    assert_eq!(before.sessions.len(), 1, "前提: セッションが 1 件ある");
    assert_eq!(before.sessions[0].completed, 1, "前提: 完了済みセッション");
    assert_eq!(before.bets.len(), 1, "前提: 買い目が 1 件ある");
    assert_eq!(before.conditions.len(), 2, "前提: 馬場入力が 2 件ある");
    assert_eq!(before.skips.len(), 1, "前提: 見送りが 1 件ある");

    run_overview(&test_app(pool.clone()), date(), 5_000, false)
        .await
        .unwrap();

    assert_eq!(
        before,
        snapshot(&pool).await,
        "--overview は予想セッション 4 テーブルを一切書き換えない（#555）"
    );
    assert_no_odds_written(&pool).await;
}

/// `--explain`（`conditional_gate_stats` を追加取得する分岐）でも同じく非干渉。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn overview_with_explain_does_not_touch_predict_session_tables(pool: PgPool) {
    let repo = PostgresRepository::new(pool.clone());
    repo.save_race(&race(date(), RACE_1, 1)).await.unwrap();
    seed_completed_session(&repo).await;

    let before = snapshot(&pool).await;

    run_overview(&test_app(pool.clone()), date(), 5_000, true)
        .await
        .unwrap();

    assert_eq!(
        before,
        snapshot(&pool).await,
        "--overview --explain も予想セッション 4 テーブルを書き換えない（#555）"
    );
    assert_no_odds_written(&pool).await;
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
    repo.save_race(&race(other_date(), RACE_1, 1))
        .await
        .unwrap();
    seed_completed_session(&repo).await;

    let before = snapshot(&pool).await;

    run_overview(&test_app(pool.clone()), date(), 5_000, false)
        .await
        .expect("開催なし日でもエラーにせず早期 return する");

    assert_eq!(
        before,
        snapshot(&pool).await,
        "開催なし日の早期 return でも予想セッション 4 テーブルを書き換えない（#555）"
    );
    assert_no_odds_written(&pool).await;
}
