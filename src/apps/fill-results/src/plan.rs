//! 照合と「入れる行」の決定（純関数）。
//!
//! 入れるのは、結果ページにあり、出馬表にもあり、DB にまだ無い馬番の行（取消・除外も status ごと入れる）。
//! 次のどれかに当たるレースは**何も書かない**（データの前提が崩れているので、人が見て決める）:
//! - 既存行の馬番が出馬表に無い、または馬名が出馬表と違う
//! - 既存行が取消・除外なのに、結果ページでは出走馬（完走・競走中止）
//! - 結果ページの出走馬が出馬表に無い（gate_num と馬名を補えない）
//! - 結果ページに同じ馬番の行が 2 つ以上ある
//! - 既存行の着順または status が、結果ページの同じ馬番の行と違う（別レースのページの混入もここで止まる）
//! - 入れた後の出走馬の行数が、結果ページの出走馬の数と合わない

use std::collections::{HashMap, HashSet};

use paddock_domain::{RaceCard, ResultStatus};
use paddock_use_case::netkeiba_scraper::ResultRow;
use rdb_gateway::ExistingResultRow;

/// 出走馬（完走・競走中止）か。取消・除外は出走していない。
pub fn is_runner(status: ResultStatus) -> bool {
    matches!(status, ResultStatus::Finished | ResultStatus::DidNotFinish)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RacePlan {
    /// DB の既存行数。
    pub existing: usize,
    /// 結果ページの出走馬の数。
    pub page_runners: usize,
    /// 入れる行（馬番順）。
    pub missing: Vec<ResultRow>,
}

impl RacePlan {
    pub fn missing_nums(&self) -> Vec<u32> {
        self.missing.iter().map(|r| r.horse_num.value()).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mismatch {
    ExistingNotInCard {
        horse_num: u32,
    },
    NameDiffers {
        horse_num: u32,
        db: String,
        card: String,
    },
    NonRunnerInDbButRanOnPage {
        horse_num: u32,
        db_status: ResultStatus,
    },
    RunnerNotInCard {
        horse_num: u32,
    },
    DuplicateOnPage {
        horse_num: u32,
    },
    ExistingDiffersFromPage {
        horse_num: u32,
        db: String,
        page: String,
    },
    RunnerCountAfter {
        page: usize,
        after: usize,
    },
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExistingNotInCard { horse_num } => {
                write!(f, "既存行の {horse_num} 番が出馬表に無い")
            }
            Self::NameDiffers {
                horse_num,
                db,
                card,
            } => write!(
                f,
                "既存行の {horse_num} 番の馬名が出馬表と違う（DB: {db} / 出馬表: {card}）"
            ),
            Self::NonRunnerInDbButRanOnPage {
                horse_num,
                db_status,
            } => write!(
                f,
                "既存行の {horse_num} 番は {db_status} なのに、結果ページでは出走馬"
            ),
            Self::RunnerNotInCard { horse_num } => {
                write!(f, "結果ページの出走馬 {horse_num} 番が出馬表に無い")
            }
            Self::ExistingDiffersFromPage {
                horse_num,
                db,
                page,
            } => write!(
                f,
                "既存行の {horse_num} 番が結果ページと違う（DB: {db} / 結果ページ: {page}）"
            ),
            Self::DuplicateOnPage { horse_num } => {
                write!(f, "結果ページに {horse_num} 番の行が 2 つ以上ある")
            }
            Self::RunnerCountAfter { page, after } => write!(
                f,
                "入れた後の出走馬の行数 {after} が結果ページの出走馬 {page} 頭と合わない"
            ),
        }
    }
}

fn describe(status: ResultStatus, pos: Option<u32>) -> String {
    match pos {
        Some(p) => format!("{status} {p} 着"),
        None => status.to_string(),
    }
}

/// 照合して入れる行を決める。照合に失敗したら、見つかった食い違いをすべて返す。
pub fn plan_race(
    card: &RaceCard,
    existing: &[ExistingResultRow],
    page: &[ResultRow],
) -> Result<RacePlan, Vec<Mismatch>> {
    let card_names: HashMap<u32, &str> = card
        .entries
        .iter()
        .map(|e| (e.horse_num.value(), e.horse_name.value()))
        .collect();
    let page_by_num: HashMap<u32, &ResultRow> =
        page.iter().map(|r| (r.horse_num.value(), r)).collect();
    let existing_nums: HashSet<u32> = existing.iter().map(|r| r.horse_num).collect();

    let mut mismatches = Vec::new();
    for row in existing {
        match card_names.get(&row.horse_num) {
            None => mismatches.push(Mismatch::ExistingNotInCard {
                horse_num: row.horse_num,
            }),
            Some(name) if *name != row.horse_name => mismatches.push(Mismatch::NameDiffers {
                horse_num: row.horse_num,
                db: row.horse_name.clone(),
                card: name.to_string(),
            }),
            Some(_) => {}
        }
        if let Some(p) = page_by_num.get(&row.horse_num) {
            let page_pos = p.finishing_position.as_ref().map(|f| f.value());
            if !is_runner(row.status) && is_runner(p.status) {
                mismatches.push(Mismatch::NonRunnerInDbButRanOnPage {
                    horse_num: row.horse_num,
                    db_status: row.status,
                });
            } else if p.status != row.status || page_pos != row.finishing_position {
                mismatches.push(Mismatch::ExistingDiffersFromPage {
                    horse_num: row.horse_num,
                    db: describe(row.status, row.finishing_position),
                    page: describe(p.status, page_pos),
                });
            }
        }
    }
    let mut seen = HashSet::new();
    for row in page {
        let n = row.horse_num.value();
        if !seen.insert(n) && !mismatches.contains(&Mismatch::DuplicateOnPage { horse_num: n }) {
            mismatches.push(Mismatch::DuplicateOnPage { horse_num: n });
        }
        if is_runner(row.status) && !card_names.contains_key(&n) {
            mismatches.push(Mismatch::RunnerNotInCard { horse_num: n });
        }
    }

    let mut missing: Vec<ResultRow> = page
        .iter()
        .filter(|r| {
            let n = r.horse_num.value();
            card_names.contains_key(&n) && !existing_nums.contains(&n)
        })
        .cloned()
        .collect();
    missing.sort_by_key(|r| r.horse_num.value());

    let page_runners = page.iter().filter(|r| is_runner(r.status)).count();
    let after = existing.iter().filter(|r| is_runner(r.status)).count()
        + missing.iter().filter(|r| is_runner(r.status)).count();
    if mismatches.is_empty() && after != page_runners {
        mismatches.push(Mismatch::RunnerCountAfter {
            page: page_runners,
            after,
        });
    }

    if !mismatches.is_empty() {
        return Err(mismatches);
    }
    Ok(RacePlan {
        existing: existing.len(),
        page_runners,
        missing,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use fixtures::*;

    /// テスト用の組み立て（日付は文字列から作り、chrono を依存に足さない）。
    pub(crate) mod fixtures {
        use paddock_domain::{
            FinishingPosition, GateNum, HorseEntry, HorseName, HorseNum, RaceCard, RaceId,
            ResultStatus, Surface, Venue,
        };
        use paddock_use_case::netkeiba_scraper::ResultRow;
        use rdb_gateway::ExistingResultRow;

        pub fn card(nums: &[u32]) -> RaceCard {
            card_for("2026-2-niigata-6-7R", nums)
        }

        pub fn card_for(race_id: &str, nums: &[u32]) -> RaceCard {
            RaceCard {
                race_id: RaceId::try_from(race_id).unwrap(),
                date: "2026-08-09".parse().unwrap(),
                post_time: None,
                venue: Venue::Niigata,
                round: 2,
                day: 6,
                race_num: 7,
                surface: Surface::Turf,
                distance: 1600,
                race_class: None,
                race_name: None,
                entries: nums
                    .iter()
                    .map(|&n| HorseEntry {
                        gate_num: GateNum::try_from(n.div_ceil(2)).unwrap(),
                        horse_num: HorseNum::try_from(n).unwrap(),
                        horse_name: HorseName::try_from(format!("ウマ{n}")).unwrap(),
                        jockey: None,
                        trainer: None,
                        weight_carried: None,
                    })
                    .collect(),
            }
        }

        pub fn db(n: u32, status: ResultStatus) -> ExistingResultRow {
            ExistingResultRow {
                horse_num: n,
                horse_name: format!("ウマ{n}"),
                status,
                finishing_position: (status == ResultStatus::Finished).then_some(n),
            }
        }

        pub fn page(n: u32, status: ResultStatus) -> ResultRow {
            ResultRow {
                horse_num: HorseNum::try_from(n).unwrap(),
                finishing_position: (status == ResultStatus::Finished)
                    .then(|| FinishingPosition::try_from(n).unwrap()),
                status,
                jockey: None,
                trainer: None,
                time_seconds: None,
                odds: None,
                horse_weight: None,
                weight_change: None,
                weight_carried: None,
                popularity: None,
            }
        }
    }

    use ResultStatus::{Cancelled, DidNotFinish, Finished, Scratched};
    use paddock_domain::FinishingPosition;

    #[test]
    fn missing_rows_are_the_page_rows_in_the_card_but_not_in_db() {
        // 1 番だけ DB にある。4 番は除外（出馬表にある）→ status ごと入れる。5 番は出馬表に無い取消 → 入れない
        let page_rows = vec![
            page(3, Finished),
            page(1, Finished),
            page(2, DidNotFinish),
            page(4, Scratched),
            page(5, Cancelled),
        ];
        let plan = plan_race(&card(&[1, 2, 3, 4]), &[db(1, Finished)], &page_rows).unwrap();
        assert_eq!(plan.missing_nums(), vec![2, 3, 4]);
        assert_eq!((plan.existing, plan.page_runners), (1, 3));
    }

    #[test]
    fn nothing_missing_is_an_empty_plan() {
        let plan = plan_race(
            &card(&[1, 2]),
            &[db(1, Finished), db(2, Finished)],
            &[page(1, Finished), page(2, Finished)],
        )
        .unwrap();
        assert!(plan.missing.is_empty());
    }

    #[test]
    fn existing_name_differing_from_card_is_a_mismatch() {
        let mut renamed = db(1, Finished);
        renamed.horse_name = "ウマ1改".into();
        let err = plan_race(
            &card(&[1, 2]),
            &[renamed],
            &[page(1, Finished), page(2, Finished)],
        )
        .unwrap_err();
        assert_eq!(
            err,
            vec![Mismatch::NameDiffers {
                horse_num: 1,
                db: "ウマ1改".into(),
                card: "ウマ1".into()
            }]
        );
    }

    #[test]
    fn existing_not_in_card_is_a_mismatch() {
        let err = plan_race(&card(&[2]), &[db(1, Finished)], &[page(2, Finished)]).unwrap_err();
        assert!(
            err.contains(&Mismatch::ExistingNotInCard { horse_num: 1 }),
            "{err:?}"
        );
    }

    #[test]
    fn non_runner_in_db_but_ran_on_page_is_a_mismatch() {
        for status in [Scratched, Cancelled] {
            let err = plan_race(
                &card(&[1, 2]),
                &[db(1, status)],
                &[page(1, Finished), page(2, Finished)],
            )
            .unwrap_err();
            assert_eq!(
                err,
                vec![Mismatch::NonRunnerInDbButRanOnPage {
                    horse_num: 1,
                    db_status: status
                }]
            );
        }
        // 両方とも非出走なら食い違いではない
        let ok = plan_race(
            &card(&[1, 2]),
            &[db(1, Scratched)],
            &[page(1, Scratched), page(2, Finished)],
        );
        assert_eq!(ok.unwrap().missing_nums(), vec![2]);
    }

    #[test]
    fn runner_not_in_card_is_a_mismatch() {
        let err = plan_race(
            &card(&[1]),
            &[db(1, Finished)],
            &[page(1, Finished), page(2, Finished)],
        )
        .unwrap_err();
        assert_eq!(err, vec![Mismatch::RunnerNotInCard { horse_num: 2 }]);
    }

    #[test]
    fn duplicate_horse_num_on_page_is_a_mismatch() {
        let err = plan_race(
            &card(&[1, 2]),
            &[db(1, Finished)],
            &[page(1, Finished), page(2, Finished), page(2, Finished)],
        )
        .unwrap_err();
        assert!(
            err.contains(&Mismatch::DuplicateOnPage { horse_num: 2 }),
            "{err:?}"
        );
    }

    #[test]
    fn existing_row_differing_from_the_page_is_a_mismatch() {
        // 着順が違う（別レースのページが混ざった場合もここで止まる）
        let mut moved = page(1, Finished);
        moved.finishing_position = Some(FinishingPosition::try_from(2u32).unwrap());
        let err = plan_race(
            &card(&[1, 2]),
            &[db(1, Finished)],
            &[moved, page(2, Finished)],
        )
        .unwrap_err();
        assert_eq!(
            err,
            vec![Mismatch::ExistingDiffersFromPage {
                horse_num: 1,
                db: "finished 1 着".into(),
                page: "finished 2 着".into()
            }]
        );
        // DB では完走、結果ページでは除外
        let err = plan_race(
            &card(&[1, 2]),
            &[db(1, Finished)],
            &[page(1, Scratched), page(2, Finished)],
        )
        .unwrap_err();
        assert!(
            matches!(
                err.as_slice(),
                [Mismatch::ExistingDiffersFromPage { horse_num: 1, .. }]
            ),
            "{err:?}"
        );
        // 取消と除外の取り違えも食い違い
        let err = plan_race(
            &card(&[1, 2]),
            &[db(1, Cancelled)],
            &[page(1, Scratched), page(2, Finished)],
        )
        .unwrap_err();
        assert!(
            matches!(
                err.as_slice(),
                [Mismatch::ExistingDiffersFromPage { horse_num: 1, .. }]
            ),
            "{err:?}"
        );
    }

    #[test]
    fn runner_count_after_must_match_the_page() {
        // DB では 1 番が完走だが、結果ページに 1 番の行が無い → 入れても出走馬が 1 頭多い
        let err = plan_race(&card(&[1, 2]), &[db(1, Finished)], &[page(2, Finished)]).unwrap_err();
        assert_eq!(err, vec![Mismatch::RunnerCountAfter { page: 1, after: 2 }]);
    }
}
