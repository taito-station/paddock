//! `live_ev_snapshots` への書き込み（#346 / ADR 0064）を Postgres（`#[sqlx::test]` の一時 DB）で
//! 往復検証する。read 経路（find_live_ev_by_date）と合わせ、upsert 冪等・複勝オッズ往復・slip JSON・
//! サイクル rank 付けを担保する。predict-watch の write パスを Python から Rust へ一本化した回帰の砦。

use chrono::NaiveDate;
use paddock_use_case::repository::{LiveEvRepository, LiveEvSnapshotRecord, SlipLegRecord};
use rdb_gateway::PostgresRepository;

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 6).unwrap()
}

/// テスト用レコード。`captured_at` / `roi` / `verdict` を可変にして冪等・rank を検証する。
fn record(captured_at: &str, roi: f64, verdict: &str) -> LiveEvSnapshotRecord {
    LiveEvSnapshotRecord {
        date: date(),
        race_id: "202602020611".to_string(),
        venue: "hakodate".to_string(),
        race_no: 11,
        post_time: Some("15:35".to_string()),
        captured_at: captured_at.to_string(),
        verdict: verdict.to_string(),
        roi,
        konsen: false,
        axis: 6,
        axis_prob: 32.5,
        axis_win_odds: Some(2.4),
        axis_place_odds_low: Some(1.1),
        axis_place_odds_high: Some(1.4),
        odds_missing: false,
        race_budget: 5000,
        legs: vec![
            SlipLegRecord {
                bet_type: "wide".to_string(),
                method: "nagashi".to_string(),
                axis: Some(6),
                combo: vec![3, 6],
                points: 1,
                amount: 1500,
            },
            SlipLegRecord {
                bet_type: "trio".to_string(),
                method: "nagashi".to_string(),
                axis: Some(6),
                combo: vec![3, 6, 8],
                points: 1,
                amount: 2000,
            },
        ],
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn snapshot_round_trips_place_and_slip(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_live_ev_snapshot(&record("2026-07-06T06:20:00Z", 104.0, "bet"))
        .await
        .unwrap();

    let rows = repo.find_live_ev_by_date(date()).await.unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.rank, 1);
    assert_eq!(r.venue, "hakodate");
    assert_eq!(r.race_no, 11);
    assert_eq!(r.post_time.as_deref(), Some("15:35"));
    assert_eq!(r.axis, 6);
    assert_eq!(r.verdict, "bet");
    assert_eq!(r.axis_win_odds, Some(2.4));
    // 複勝オッズ帯が往復する（#346 の主眼）。
    assert_eq!(r.axis_place_odds_low, Some(1.1));
    assert_eq!(r.axis_place_odds_high, Some(1.4));

    // slip JSONB が read 側 SlipView 契約（race_budget / legs[bet_type,method,axis,combo,points,amount]）で往復する。
    let slip: serde_json::Value = serde_json::from_str(&r.slip_json).unwrap();
    assert_eq!(slip["race_budget"], 5000);
    let legs = slip["legs"].as_array().unwrap();
    assert_eq!(legs.len(), 2);
    assert_eq!(legs[0]["bet_type"], "wide");
    assert_eq!(legs[0]["method"], "nagashi");
    assert_eq!(legs[0]["axis"], 6);
    assert_eq!(legs[0]["combo"], serde_json::json!([3, 6]));
    assert_eq!(legs[0]["points"], 1);
    assert_eq!(legs[0]["amount"], 1500);
    assert_eq!(legs[1]["bet_type"], "trio");
    assert_eq!(legs[1]["combo"], serde_json::json!([3, 6, 8]));
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn upsert_is_idempotent_on_race_and_captured_at(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let cap = "2026-07-06T06:20:00Z";

    // 同一サイクル（同一 race_id, captured_at）を 2 度書く＝cron 二重発火・手動再走。
    repo.save_live_ev_snapshot(&record(cap, 92.0, "skip"))
        .await
        .unwrap();
    repo.save_live_ev_snapshot(&record(cap, 130.0, "bet"))
        .await
        .unwrap();

    // 行は増えず（ON CONFLICT DO UPDATE）、最後の値で上書きされる。
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM live_ev_snapshots")
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "同一 (race_id, captured_at) は 1 行に畳まれる");

    let rows = repo.find_live_ev_by_date(date()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].verdict, "bet");
    assert!((rows[0].roi - 130.0).abs() < 1e-9);
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn two_cycles_are_ranked_latest_first(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);

    // 別 captured_at の 2 サイクル。find は captured_at 降順で rank=1（最新）/2（直前）を返す。
    repo.save_live_ev_snapshot(&record("2026-07-06T06:20:00Z", 92.0, "skip"))
        .await
        .unwrap();
    repo.save_live_ev_snapshot(&record("2026-07-06T06:25:00Z", 130.0, "bet"))
        .await
        .unwrap();

    let rows = repo.find_live_ev_by_date(date()).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].rank, 1);
    assert_eq!(rows[0].captured_at, "2026-07-06T06:25:00Z");
    assert_eq!(rows[0].verdict, "bet");
    assert_eq!(rows[1].rank, 2);
    assert_eq!(rows[1].captured_at, "2026-07-06T06:20:00Z");
    assert_eq!(rows[1].verdict, "skip");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn place_odds_null_when_absent(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // JRA 未公開で複勝欠落＝None を書いて NULL で往復する（read 側は「複勝—」表示に落とす）。
    let mut rec = record("2026-07-06T06:20:00Z", 100.0, "bet");
    rec.axis_place_odds_low = None;
    rec.axis_place_odds_high = None;
    rec.axis_win_odds = None;
    repo.save_live_ev_snapshot(&rec).await.unwrap();

    let rows = repo.find_live_ev_by_date(date()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].axis_win_odds, None);
    assert_eq!(rows[0].axis_place_odds_low, None);
    assert_eq!(rows[0].axis_place_odds_high, None);
}
