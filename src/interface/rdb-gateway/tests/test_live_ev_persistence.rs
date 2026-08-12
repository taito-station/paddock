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
        roughness: 0.72,
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
    // bool 列（konsen / odds_missing）も書いた値どおり往復する。
    assert!(!r.konsen);
    assert!(!r.odds_missing);
    // 荒れ度も往復する（#344）。
    assert_eq!(r.roughness, Some(0.72));

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

    // raw アーカイブ列も識別子・時刻・slip を漏れなく保持する（read パス非依存の後方互換砦）。
    // JSONB は `::text` で取り出す（workspace sqlx は `json` feature 無効）。
    let raw_text: String =
        sqlx::query_scalar("SELECT raw::text FROM live_ev_snapshots WHERE race_id = $1")
            .bind("202602020611")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
    let raw: serde_json::Value = serde_json::from_str(&raw_text).unwrap();
    assert_eq!(raw["date"], "2026-07-06");
    assert_eq!(raw["captured_at"], "2026-07-06T06:20:00Z");
    assert_eq!(raw["post_time"], "15:35");
    assert_eq!(raw["race_id"], "202602020611");
    assert_eq!(raw["axis_place_odds_low"], 1.1);
    assert_eq!(raw["slip"]["legs"].as_array().unwrap().len(), 2);
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
async fn konsen_box_leg_round_trips_with_null_axis(pool: sqlx::PgPool) {
    // 混戦（#352）: konsen=true と、印馬3連複ボックスの leg（method="box"・axis=None）が
    // slip JSONB を往復する。box は軸を持たないため axis は JSON null で保存・復元される。
    let repo = PostgresRepository::new(pool);
    let mut rec = record("2026-07-06T06:20:00Z", 110.0, "bet");
    rec.konsen = true;
    rec.legs.push(SlipLegRecord {
        bet_type: "trio".to_string(),
        method: "box".to_string(),
        axis: None,
        combo: vec![3, 6, 8],
        points: 1,
        amount: 1500,
    });
    repo.save_live_ev_snapshot(&rec).await.unwrap();

    let rows = repo.find_live_ev_by_date(date()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].konsen, "混戦フラグが往復する");

    let slip: serde_json::Value = serde_json::from_str(&rows[0].slip_json).unwrap();
    let legs = slip["legs"].as_array().unwrap();
    let bx = legs
        .iter()
        .find(|l| l["method"] == "box")
        .expect("box leg があるはず");
    assert_eq!(bx["bet_type"], "trio");
    assert!(bx["axis"].is_null(), "box は軸なし＝JSON null");
    assert_eq!(bx["combo"], serde_json::json!([3, 6, 8]));
    assert_eq!(bx["amount"], 1500);
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

// --- 買い目固定の読み出し（#601 軸ロック） -------------------------------------

/// 相手・混戦 band を持つレコードを組む。`method` / `bet_type` の組み合わせから
/// 固定（軸・相手・band）を復元できるかを見るためのフィクスチャ。
fn record_with_legs(
    captured_at: &str,
    axis: u32,
    partners: &[u32],
    box_horses: &[u32],
) -> LiveEvSnapshotRecord {
    let mut rec = record(captured_at, 90.0, "skip");
    rec.axis = axis;
    rec.konsen = box_horses.len() >= 4;
    rec.legs = Vec::new();
    // 馬連・ワイドは「軸×相手」のながし。相手はこの 2 券種の和集合から復元される。
    for (i, p) in partners.iter().enumerate() {
        let mut combo = vec![axis, *p];
        combo.sort_unstable();
        // ワイド脚を 1 本だけ意図的に落とし、片方の券種が予算端数で欠けても
        // もう片方から相手を拾えることを確かめる。
        if i != 0 {
            rec.legs.push(SlipLegRecord {
                bet_type: "wide".to_string(),
                method: "nagashi".to_string(),
                axis: Some(axis),
                combo: combo.clone(),
                points: 1,
                amount: 300,
            });
        }
        rec.legs.push(SlipLegRecord {
            bet_type: "quinella".to_string(),
            method: "nagashi".to_string(),
            axis: Some(axis),
            combo,
            points: 1,
            amount: 300,
        });
    }
    // 印馬 3 連複ボックス（混戦時のみ）。band はこの脚の和集合から復元される。
    for i in 0..box_horses.len() {
        for j in (i + 1)..box_horses.len() {
            for k in (j + 1)..box_horses.len() {
                let mut combo = vec![box_horses[i], box_horses[j], box_horses[k]];
                combo.sort_unstable();
                rec.legs.push(SlipLegRecord {
                    bet_type: "trio".to_string(),
                    method: "box".to_string(),
                    axis: None,
                    combo,
                    points: 1,
                    amount: 100,
                });
            }
        }
    }
    rec
}

/// 固定は **その日の初回スイープ**から採る。最新（`find_live_ev_by_date` が返す方）ではない。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn pins_return_the_earliest_sweep_not_the_latest(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    // 3 スイープ。時刻が進むにつれ軸も相手も動く＝#601 が直そうとしている現象そのもの。
    for (at, axis, partners) in [
        ("2026-07-06T00:10:00Z", 6, [3, 8, 11]),
        ("2026-07-06T05:20:00Z", 9, [1, 2, 3]),
        ("2026-07-06T06:20:00Z", 9, [1, 2, 5]),
    ] {
        repo.save_live_ev_snapshot(&record_with_legs(at, axis, &partners, &[]))
            .await
            .unwrap();
    }

    let pins = repo.find_live_ev_pins_by_date(date()).await.unwrap();
    assert_eq!(pins.len(), 1, "レースごとに 1 件");
    let pin = &pins[0];
    assert_eq!(pin.race_id, "202602020611");
    assert_eq!(pin.axis, 6, "最古（初回スイープ）の軸。最新の 9 ではない");
    assert_eq!(
        pin.partners,
        vec![3, 8, 11],
        "最古の相手（馬番昇順）。ワイド脚を 1 本落としても馬連側で拾える"
    );
    assert_eq!(pin.captured_at, "2026-07-06T00:10:00Z");
    assert!(pin.konsen_band.is_empty(), "box 脚が無い＝非混戦で固定");
}

/// 混戦だった初回スイープは box 脚から印馬 band を復元する（配分ごと固定するために要る）。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn pins_restore_konsen_band_from_box_legs(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_live_ev_snapshot(&record_with_legs(
        "2026-07-06T00:10:00Z",
        6,
        &[3, 8, 11],
        &[6, 3, 8, 11],
    ))
    .await
    .unwrap();

    let pins = repo.find_live_ev_pins_by_date(date()).await.unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(
        pins[0].konsen_band,
        vec![3, 6, 8, 11],
        "box 脚の和集合＝印馬 band（馬番昇順）"
    );
}

/// まだ 1 度も評価していない日は固定なし＝そのスイープで選定が決まる。
#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn pins_are_empty_when_no_sweep_recorded(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let pins = repo.find_live_ev_pins_by_date(date()).await.unwrap();
    assert!(pins.is_empty());
}
