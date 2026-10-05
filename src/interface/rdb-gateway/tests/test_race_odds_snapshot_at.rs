//! `find_race_odds_snapshot`（#724: 時点を指定して `race_odds_snapshots` から 1 スナップショットを読む）を
//! Postgres で検証する。時点の比較は `timestamptz` で行うこと（`fetched_at` の `...+00:00` と `...Z` の
//! 書式の混在で、文字列の辞書順だと同じ秒の付近の順序が崩れる）を担保する。

use chrono::{DateTime, TimeZone, Utc};
use paddock_domain::{HorseNum, OrderedPair, OrderedTriple, Pair, RaceId, Triple};
use paddock_use_case::repository::{OddsRepository, OddsRow, RaceOddsRecord, SnapshotPoint};
use rdb_gateway::PostgresRepository;

fn race_id() -> RaceId {
    RaceId::try_from("2026-2-niigata-3-10R").unwrap()
}

fn horse(n: u32) -> HorseNum {
    HorseNum::try_from(n).unwrap()
}

fn at(h: u32, m: u32, s: u32, ms: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 15, h, m, s).unwrap() + chrono::Duration::milliseconds(ms as i64)
}

fn win_row(odds: f64) -> OddsRow {
    OddsRow {
        bet_type: "win".to_string(),
        combination_key: "1".to_string(),
        odds,
        odds_high: None,
        popularity: None,
    }
}

async fn save(repo: &PostgresRepository, fetched_at: DateTime<Utc>, rows: Vec<OddsRow>) {
    repo.save_race_odds(&RaceOddsRecord {
        race_id: race_id(),
        fetched_at,
        rows,
    })
    .await
    .unwrap();
}

/// 単勝だけのスナップショット（odds-collect の単複スイープ相当）。
async fn save_win(repo: &PostgresRepository, fetched_at: DateTime<Utc>, odds: f64) {
    save(repo, fetched_at, vec![win_row(odds)]).await;
}

/// 全券種そろったスナップショット（predict-watch の refresh・fetch-card 相当）。
async fn save_complete(repo: &PostgresRepository, fetched_at: DateTime<Utc>, win1: f64) {
    let pair = Pair::try_from((horse(1), horse(2))).unwrap();
    let opair = OrderedPair::try_from((horse(2), horse(1))).unwrap();
    let triple = Triple::try_from((horse(1), horse(2), horse(3))).unwrap();
    let otriple = OrderedTriple::try_from((horse(1), horse(2), horse(3))).unwrap();
    save(
        repo,
        fetched_at,
        vec![
            win_row(win1),
            OddsRow::quinella(pair, 12.4),
            OddsRow::wide(pair, 3.1, 4.8),
            OddsRow::exacta(opair, 25.0),
            OddsRow::trio(triple, 88.0),
            OddsRow::trifecta(otriple, 410.0),
        ],
    )
    .await;
}

/// `...Z` 書式の `fetched_at` で単勝 1 行を直接入れる（`live_ev_snapshots.captured_at` と同じ書式）。
async fn insert_win_z(repo: &PostgresRepository, fetched_at_z: &str, odds: f64) {
    sqlx::query(
        "INSERT INTO race_odds_snapshots (race_id, bet_type, combination_key, odds, fetched_at) \
         VALUES ($1, 'win', '1', $2, $3)",
    )
    .bind(race_id().value())
    .bind(odds)
    .bind(fetched_at_z)
    .execute(&repo.pool)
    .await
    .unwrap();
}

fn win1(o: &paddock_use_case::repository::SnapshotOdds) -> f64 {
    o.odds.win.get(&horse(1)).unwrap().value()
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn first_win_since_skips_earlier_and_takes_first_after(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 前日夜の fetch-card（JST 0 時より前）は朝に採らない
    save_win(
        &repo,
        Utc.with_ymd_and_hms(2026, 8, 14, 11, 27, 0).unwrap(),
        2.0,
    )
    .await;
    save_win(&repo, at(0, 30, 0, 0), 3.0).await;
    save_win(&repo, at(1, 0, 0, 0), 4.0).await;
    let since = Utc.with_ymd_and_hms(2026, 8, 14, 15, 0, 0).unwrap(); // JST 08-15 00:00
    let got = repo
        .find_race_odds_snapshot(&race_id(), SnapshotPoint::FirstWinSince(since))
        .await
        .unwrap()
        .expect("since 以降の単勝がある");
    assert!(
        (win1(&got) - 3.0).abs() < 1e-9,
        "JST 0 時以降で最初の 00:30 を採る"
    );
    assert!(got.fetched_at.starts_with("2026-08-15T00:30:00"));
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn first_win_since_orders_by_time_not_by_text(pool: sqlx::PgPool) {
    // 同じ秒の `...20Z`（書式 Z）と `...20.500+00:00`。時刻では前者が先だが、文字列の辞書順では
    // '.'(0x2E) < 'Z'(0x5A) なので後者が先に並ぶ。時刻で比べていれば 20Z の 7.0 を採る。
    let repo = PostgresRepository::new(pool);
    insert_win_z(&repo, "2026-08-15T05:07:20Z", 7.0).await;
    save_win(&repo, at(5, 7, 20, 500), 9.0).await;
    let got = repo
        .find_race_odds_snapshot(&race_id(), SnapshotPoint::FirstWinSince(at(5, 0, 0, 0)))
        .await
        .unwrap()
        .expect("単勝がある");
    assert!((win1(&got) - 7.0).abs() < 1e-9, "時刻順で先の 20Z を採る");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn first_complete_since_takes_the_sweep_saved_after_captured_at(pool: sqlx::PgPool) {
    // 初回スイープ: captured_at = 05:07:17Z。その前後に odds-collect の単複、05:08:18.5 に
    // predict-watch の refresh が全券種を保存した。T-40 の固定に使うのは 05:08:18.5 の盤。
    let repo = PostgresRepository::new(pool);
    save_complete(&repo, at(4, 0, 0, 0), 1.5).await; // captured_at より前の盤（採らない）
    save_win(&repo, at(5, 7, 0, 0), 2.5).await;
    save_win(&repo, at(5, 7, 30, 0), 3.5).await; // 後だが単勝だけ（complete でない）
    save_complete(&repo, at(5, 8, 18, 500), 4.5).await;
    save_complete(&repo, at(5, 13, 0, 0), 5.5).await;
    let got = repo
        .find_race_odds_snapshot(
            &race_id(),
            SnapshotPoint::FirstCompleteSince(at(5, 7, 17, 0)),
        )
        .await
        .unwrap()
        .expect("captured_at 以降に全券種の盤がある");
    assert!(
        (win1(&got) - 4.5).abs() < 1e-9,
        "captured_at 以降で最初の complete"
    );
    assert!(got.odds.is_complete());
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn first_complete_since_includes_sub_second_after_whole_second_since(pool: sqlx::PgPool) {
    // since が秒ちょうど、盤が 0.5 秒後。時刻では「以降」に入る。
    let repo = PostgresRepository::new(pool);
    save_complete(&repo, at(5, 7, 17, 500), 4.5).await;
    let got = repo
        .find_race_odds_snapshot(
            &race_id(),
            SnapshotPoint::FirstCompleteSince(at(5, 7, 17, 0)),
        )
        .await
        .unwrap();
    assert!(got.is_some_and(|o| (win1(&o) - 4.5).abs() < 1e-9));
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn returns_none_when_nothing_after_since(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    save_complete(&repo, at(4, 0, 0, 0), 1.5).await;
    let point = SnapshotPoint::FirstCompleteSince(at(5, 0, 0, 0));
    assert!(
        repo.find_race_odds_snapshot(&race_id(), point)
            .await
            .unwrap()
            .is_none()
    );
    let point = SnapshotPoint::FirstWinSince(at(5, 0, 0, 0));
    assert!(
        repo.find_race_odds_snapshot(&race_id(), point)
            .await
            .unwrap()
            .is_none()
    );
}
