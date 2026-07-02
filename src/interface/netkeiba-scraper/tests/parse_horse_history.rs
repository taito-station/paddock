use netkeiba_scraper::parse::parse_horse_history;
use paddock_domain::{ResultStatus, Surface, Venue};

const FIXTURE: &str = include_str!("fixtures/horse_result.html");

// fixture はシャフリヤール（2018105165）の競走戦績テーブル。全 17 走中、海外 5 走は
// race_id リンクを持たずスキップされ、JRA 平地 12 走だけが残る。
#[test]
fn keeps_only_jra_rows_and_skips_overseas() {
    let runs = parse_horse_history(FIXTURE).expect("parse");
    assert_eq!(
        runs.len(),
        12,
        "JRA 平地のみ（海外は race_id 無しでスキップ）"
    );
    // 全行 venue が JRA10場のいずれか、距離は正、horse_name は固定。
    for r in &runs {
        assert_eq!(r.horse_name.value(), "シャフリヤール");
        assert!(r.distance >= 1000);
        assert!(matches!(r.surface, Surface::Turf | Surface::Dirt));
    }
}

#[test]
fn parses_sapporo_kinen_row_exactly() {
    let runs = parse_horse_history(FIXTURE).expect("parse");
    // 2024/08/18 札幌記念(GII) 5着、race_id 202401020411。
    let r = runs
        .iter()
        .find(|r| r.netkeiba_race_id == "202401020411")
        .expect("札幌記念の行");

    assert_eq!(r.date.to_string(), "2024-08-18");
    assert_eq!(r.venue, Venue::Sapporo);
    assert_eq!(r.round, 2);
    assert_eq!(r.day, 4);
    assert_eq!(r.race_num, 11);
    assert_eq!(r.surface, Surface::Turf);
    assert_eq!(r.distance, 2000);
    assert_eq!(r.track_condition.as_ref().map(|t| t.as_str()), Some("良"));
    assert_eq!(r.status, ResultStatus::Finished);
    assert_eq!(r.finishing_position.as_ref().map(|p| p.value()), Some(5));
    assert_eq!(r.gate_num.value(), 4);
    assert_eq!(r.horse_num.value(), 4);
    assert_eq!(r.jockey.as_ref().map(|j| j.value()), Some("武豊"));
    assert_eq!(r.weight_carried, Some(58.0));
    assert_eq!(r.popularity, Some(2));
    assert_eq!(r.odds, Some(7.6));
    assert_eq!(r.margin.as_deref(), Some("0.6"));
    assert_eq!(r.horse_weight, Some(464));
    assert_eq!(r.weight_change, Some(0));
    // タイム 2:00.2 = 120.2 秒
    let secs = r.time_seconds.as_ref().map(|t| t.value()).unwrap();
    assert!((secs - 120.2).abs() < 1e-6, "got {secs}");
    // #329 Phase0: レース名(列4)・通過順位(列25)。
    assert_eq!(r.race_name.as_deref(), Some("札幌記念(GII)"));
    assert_eq!(r.corner_positions.as_deref(), Some("5-3-6-6"));
    // #329 Phase1: 出走頭数(列6)。脚質の相対化分母。
    assert_eq!(r.field_size, Some(11));
}

#[test]
fn parses_nakayama_row_with_weight() {
    let runs = parse_horse_history(FIXTURE).expect("parse");
    // 2023/12/24 有馬記念 中山 race_id 202306050811、馬体重 454(0)。
    let r = runs
        .iter()
        .find(|r| r.netkeiba_race_id == "202306050811")
        .expect("中山の行");
    assert_eq!(r.venue, Venue::Nakayama);
    assert_eq!(r.distance, 2500);
    assert_eq!(r.finishing_position.as_ref().map(|p| p.value()), Some(5));
    assert_eq!(r.horse_weight, Some(454));
    assert_eq!(r.weight_change, Some(0));
    assert_eq!(r.race_name.as_deref(), Some("有馬記念(GI)"));
    assert_eq!(r.corner_positions.as_deref(), Some("4-4-5-6"));
    // #329 Phase1: 出走頭数(列6)。
    assert_eq!(r.field_size, Some(16));
}
