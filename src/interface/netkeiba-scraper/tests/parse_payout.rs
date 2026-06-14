use netkeiba_scraper::parse::parse_race_payouts;
use paddock_domain::RaceId;

const FIXTURE: &str = include_str!("fixtures/race_result.html");

fn race_id() -> RaceId {
    // 202606030801 = 2026 3回中山8日1R。result test と同じ fixture。
    RaceId::try_from("2026-3-nakayama-8-1R").unwrap()
}

// fixture の払戻ブロック（実値）:
//   単勝 6=2,270 / 複勝 6=280,4=110,11=150
//   馬連 4-6=1,170 / ワイド 4-6=430,6-11=980,4-11=180
//   馬単 6>4=3,830 / 3連複 4-6-11=1,740 / 3連単 6>4>11=22,000
//   枠連 4-5=1,200 は predict 非対象でスキップされる。
#[test]
fn parses_all_predict_bet_types() {
    let p = parse_race_payouts(FIXTURE, race_id()).expect("parse payouts");
    assert!(!p.is_empty(), "確定済み fixture は払戻を持つ");

    // 単勝
    assert_eq!(p.payoff("win", "6"), Some(2270));
    // 複勝（馬番ごとに 1 点）
    assert_eq!(p.payoff("place", "6"), Some(280));
    assert_eq!(p.payoff("place", "4"), Some(110));
    assert_eq!(p.payoff("place", "11"), Some(150));
    // 馬連（昇順）
    assert_eq!(p.payoff("quinella", "4-6"), Some(1170));
    // ワイド（昇順・3 組）
    assert_eq!(p.payoff("wide", "4-6"), Some(430));
    assert_eq!(p.payoff("wide", "6-11"), Some(980));
    assert_eq!(p.payoff("wide", "4-11"), Some(180));
    // 馬単（着順、> 連結）
    assert_eq!(p.payoff("exacta", "6>4"), Some(3830));
    // 3連複（昇順）
    assert_eq!(p.payoff("trio", "4-6-11"), Some(1740));
    // 3連単（着順）
    assert_eq!(p.payoff("trifecta", "6>4>11"), Some(22000));
}

#[test]
fn skips_wakuren_and_misses() {
    let p = parse_race_payouts(FIXTURE, race_id()).expect("parse payouts");
    // 枠連は predict 非対象なのでどの券種にも入らない（type_label を持たない）。
    // 念のため馬連キーで枠連の値(1200)が紛れていないことを確認する。
    assert_ne!(p.payoff("quinella", "4-5"), Some(1200));
    // 不的中の組合せは None。
    assert_eq!(p.payoff("quinella", "1-2"), None);
    assert_eq!(p.payoff("win", "1"), None);
}

#[test]
fn empty_html_is_unconfirmed() {
    let p = parse_race_payouts("<html><body></body></html>", race_id()).expect("parse");
    assert!(p.is_empty());
}

// 組合せ数と配当数が食い違う行は、誤った馬番に配当を貼らないよう当該券種ごと skip する
// （複勝・ワイドのように 1 行に複数組×複数配当が並ぶ券種での構造ズレへの保険）。
#[test]
fn mismatched_combo_and_payout_count_is_skipped() {
    // 馬連: 組合せ 1 組（4-6）に対し配当が 2 つ → 不一致なので採用しない。
    let html = r#"<table class="Payout_Detail_Table"><tbody>
        <tr class="Umaren"><th>馬連</th>
          <td class="Result"><ul><li><span>4</span></li><li><span>6</span></li></ul></td>
          <td class="Payout"><span>1,170円<br />999円</span></td>
        </tr></tbody></table>"#;
    let p = parse_race_payouts(html, race_id()).expect("parse");
    assert_eq!(p.payoff("quinella", "4-6"), None, "件数不一致の行は採用しない");
    assert!(p.is_empty());
}

// 区切り無しで配当が連結（`280円110円`）しても 1 配当ずつ正しく切り出す。
#[test]
fn concatenated_payouts_split_on_yen() {
    // 複勝: 馬番 6,4 の 2 頭に対し配当が `280円110円`（br 無し連結）でも [280,110] に分割される。
    let html = r#"<table class="Payout_Detail_Table"><tbody>
        <tr class="Fukusho"><th>複勝</th>
          <td class="Result"><div><span>6</span></div><div><span>4</span></div></td>
          <td class="Payout"><span>280円110円</span></td>
        </tr></tbody></table>"#;
    let p = parse_race_payouts(html, race_id()).expect("parse");
    assert_eq!(p.payoff("place", "6"), Some(280));
    assert_eq!(p.payoff("place", "4"), Some(110));
}
