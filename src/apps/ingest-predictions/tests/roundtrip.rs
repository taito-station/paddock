//! 予想の保存→取得ラウンドトリップと upsert 冪等性を Postgres（#[sqlx::test] の一時DB）で検証する。

use chrono::{NaiveDate, Utc};
use paddock_domain::{
    Mark, PadPrediction, PredictionBet, PredictionHorse, PredictionResult, Venue,
};
use paddock_use_case::repository::PadPredictionRepository;
use rdb_gateway::PostgresRepository;

fn sample() -> PadPrediction {
    PadPrediction {
        date: NaiveDate::from_ymd_opt(2026, 6, 13).unwrap(),
        venue: Venue::Hanshin,
        race_num: 4,
        title: Some("3歳未勝利".into()),
        budget: Some(10000),
        strategy_note: Some("人気軸＋相手広め".into()),
        commentary: Some("中位人気の相手抜けが反省点".into()),
        horses: vec![
            PredictionHorse {
                horse_num: 7,
                horse_name: "ラパンドール".into(),
                jockey: Some("松山".into()),
                mark: Some(Mark::Honmei),
                win_odds: Some(2.4),
                popularity: Some(1),
                win_prob: Some(25.4),
                place_prob: Some(25.4),
                show_prob: Some(25.4),
                comment: Some("単独最上位".into()),
            },
            PredictionHorse {
                horse_num: 4,
                horse_name: "ファランギーナ".into(),
                jockey: Some("松若".into()),
                mark: Some(Mark::Renge),
                win_odds: Some(13.2),
                popularity: Some(6),
                win_prob: Some(6.1),
                place_prob: Some(15.2),
                show_prob: Some(21.4),
                comment: None,
            },
        ],
        bets: vec![
            PredictionBet {
                bet_type: "単勝".into(),
                combination: "7".into(),
                amount: 600,
            },
            PredictionBet {
                bet_type: "馬連".into(),
                combination: "7-14".into(),
                amount: 1000,
            },
        ],
        result: Some(PredictionResult {
            finish: [Some(7), Some(4), Some(13)],
            recovery_rate: Some(52.1),
            pnl: Some(-4790),
            note: Some("印は上位3頭を捕捉".into()),
        }),
    }
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn save_find_roundtrip(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let pred = sample();
    repo.save_pad_prediction(&pred, Utc::now()).await.unwrap();

    let got = repo
        .find_pad_prediction(pred.date, pred.venue, pred.race_num)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(got.title.as_deref(), Some("3歳未勝利"));
    assert_eq!(got.budget, Some(10000));
    // 馬は horse_num 昇順で返る（4 → 7）。
    assert_eq!(got.horses.len(), 2);
    assert_eq!(got.horses[0].horse_num, 4);
    assert_eq!(got.horses[0].horse_name, "ファランギーナ");
    assert_eq!(got.horses[0].mark, Some(Mark::Renge));
    assert_eq!(got.horses[1].horse_num, 7);
    assert_eq!(got.horses[1].horse_name, "ラパンドール");
    assert_eq!(got.horses[1].mark, Some(Mark::Honmei));
    assert_eq!(got.horses[1].win_prob, Some(25.4));
    assert_eq!(got.bets.len(), 2);
    assert_eq!(got.bets[1].combination, "7-14");
    let r = got.result.unwrap();
    assert_eq!(r.finish, [Some(7), Some(4), Some(13)]);
    assert_eq!(r.pnl, Some(-4790));
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn re_save_is_idempotent_and_replaces_children(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    repo.save_pad_prediction(&sample(), Utc::now())
        .await
        .unwrap();

    // 子を減らして同キーで再保存 → 重複せず置き換わる。
    let mut p2 = sample();
    p2.horses.truncate(1);
    p2.bets.clear();
    repo.save_pad_prediction(&p2, Utc::now()).await.unwrap();

    // 予想ヘッダが重複していないこと（同キー再保存で 1 行のまま）。
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM predictions")
        .fetch_one(&repo.pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "同キー再保存で予想が重複してはならない");

    // 子行が置き換わっていること。
    let got = repo
        .find_pad_prediction(p2.date, p2.venue, p2.race_num)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.horses.len(), 1, "馬の子行が置き換わる");
    assert_eq!(got.bets.len(), 0, "買い目の子行が置き換わる");
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn race_id_resolves_and_is_preserved_on_reingest(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);

    // races に一致行を入れて保存 → race_id が解決される。
    sqlx::query(
        "INSERT INTO races (race_id,date,venue,round,day,race_num,surface,distance,source) \
         VALUES ('R-TEST','2026-06-13','阪神',3,4,4,'芝',1800,'pdf')",
    )
    .execute(&repo.pool)
    .await
    .unwrap();
    repo.save_pad_prediction(&sample(), Utc::now())
        .await
        .unwrap();

    let rid: Option<String> = sqlx::query_scalar(
        "SELECT race_id FROM predictions WHERE date='2026-06-13' AND venue='阪神' AND race_num=4",
    )
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(rid.as_deref(), Some("R-TEST"));

    // races を消して未解決状態で再 ingest → 解決済み race_id は COALESCE で保持される。
    sqlx::query("DELETE FROM races")
        .execute(&repo.pool)
        .await
        .unwrap();
    repo.save_pad_prediction(&sample(), Utc::now())
        .await
        .unwrap();

    let rid2: Option<String> = sqlx::query_scalar(
        "SELECT race_id FROM predictions WHERE date='2026-06-13' AND venue='阪神' AND race_num=4",
    )
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(
        rid2.as_deref(),
        Some("R-TEST"),
        "一度解決した race_id は未解決の再取込で巻き戻らない"
    );
}

#[sqlx::test(migrations = "../../../deployments/db/migrations")]
async fn missing_prediction_is_none(pool: sqlx::PgPool) {
    let repo = PostgresRepository::new(pool);
    let got = repo
        .find_pad_prediction(
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            Venue::Tokyo,
            1,
        )
        .await
        .unwrap();
    assert!(got.is_none());
}
