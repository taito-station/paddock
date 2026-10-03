//! 保存した結果ページから DB に書く（ネットワーク不使用）。
//!
//! レースごとに HTML を読み、races 行・出馬表・既存行を読んで照合し（[`crate::plan`]）、足りない行だけを INSERT する。
//! 照合に失敗したレース・HTML が無いレース・races 行や出馬表が無いレースは**書かずに**失敗として数え、他のレースは続ける
//! （呼び出し側は失敗が 1 件でもあれば非ゼロで終える）。`dry_run` では何も書かないが、書かない判定は本番と同じに行う。
//! DB の読み書き自体の失敗（接続断など）は、そのレースを失敗に数えて以降を処理せずに止める（それまでの結果は返す）。

use std::future::Future;
use std::path::Path;

use fetch_final_odds::targets::Target;
use netkeiba_scraper::parse::parse_race_result;
use paddock_domain::{RaceCard, RaceId};
use paddock_use_case::netkeiba_scraper::ResultRow;
use paddock_use_case::repository::{RaceCardRepository, RaceRepository};
use rdb_gateway::{ExistingResultRow, PostgresRepository};

use crate::fetch::html_path;
use crate::plan::plan_race;

/// DB の読み書き口（テストで差し替える）。
pub trait FillStore {
    fn race_exists(&self, race_id: &RaceId) -> impl Future<Output = anyhow::Result<bool>>;
    fn race_card(&self, race_id: &RaceId)
    -> impl Future<Output = anyhow::Result<Option<RaceCard>>>;
    fn existing(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = anyhow::Result<Vec<ExistingResultRow>>>;
    fn insert(
        &self,
        card: &RaceCard,
        rows: &[ResultRow],
    ) -> impl Future<Output = anyhow::Result<Vec<u32>>>;
}

impl FillStore for PostgresRepository {
    async fn race_exists(&self, race_id: &RaceId) -> anyhow::Result<bool> {
        Ok(RaceRepository::race_exists(self, race_id).await?)
    }
    async fn race_card(&self, race_id: &RaceId) -> anyhow::Result<Option<RaceCard>> {
        Ok(self.find_race_card(race_id).await?)
    }
    async fn existing(&self, race_id: &RaceId) -> anyhow::Result<Vec<ExistingResultRow>> {
        Ok(self.find_result_rows_for_fill(race_id).await?)
    }
    async fn insert(&self, card: &RaceCard, rows: &[ResultRow]) -> anyhow::Result<Vec<u32>> {
        Ok(self.insert_missing_results(card, rows).await?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RaceReport {
    pub race_id: String,
    pub existing: usize,
    pub page_runners: usize,
    /// 入れる（dry-run では入れるはずの）馬番。
    pub planned: Vec<u32>,
    /// 実際に入れた馬番（dry-run では `None`）。
    pub inserted: Option<Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ApplyReport {
    pub races: Vec<RaceReport>,
    /// 失敗したレースと理由（書かなかったのか、書いた後で食い違ったのかは理由に書く）。
    pub failures: Vec<(String, String)>,
}

/// 接続先の DB 名が指定と一致するか（隔離 DB と共有 DB の取り違えを防ぐ）。
pub fn check_db_name(actual: &str, expected: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        actual == expected,
        "接続先の DB は {actual} です（--db-name {expected} と違うので書きません）。PADDOCK_DB_URL を確かめてください"
    );
    Ok(())
}

/// 対象を順に処理する。DB の読み書き自体が失敗したら、そのレースを失敗に数えて止める（それまでの結果は返す）。
pub async fn run(
    store: &impl FillStore,
    targets: &[Target],
    out_dir: &Path,
    dry_run: bool,
) -> ApplyReport {
    let mut report = ApplyReport::default();
    for t in targets {
        if let Err(e) = apply_one(store, t, out_dir, dry_run, &mut report).await {
            report.failures.push((
                t.race_id.clone(),
                format!("DB の読み書きに失敗したので止めました（以降のレースは未処理）: {e:#}"),
            ));
            break;
        }
    }
    report
}

/// 1 レース分。照合などで書かないと決めたものは `report.failures` に積んで `Ok`、DB の失敗だけ `Err`。
async fn apply_one(
    store: &impl FillStore,
    t: &Target,
    out_dir: &Path,
    dry_run: bool,
    report: &mut ApplyReport,
) -> anyhow::Result<()> {
    let mut fail = |why: String| {
        report
            .failures
            .push((t.race_id.clone(), format!("書かなかった: {why}")));
    };
    let html = match std::fs::read_to_string(html_path(out_dir, &t.race_id)) {
        Ok(h) => h,
        Err(e) => {
            fail(format!("結果ページの HTML を読めません: {e}"));
            return Ok(());
        }
    };
    let page = match parse_race_result(&html, &t.netkeiba_id) {
        Ok(p) => p,
        Err(e) => {
            fail(format!("結果ページを読めません: {e}"));
            return Ok(());
        }
    };
    let race_id = match RaceId::try_from(t.race_id.as_str()) {
        Ok(id) => id,
        Err(e) => {
            fail(format!("race_id が不正です: {e}"));
            return Ok(());
        }
    };
    // races 行の有無は INSERT 側でも確かめるが、dry-run でも同じ判定になるようここで見る
    if !store.race_exists(&race_id).await? {
        fail("races 行がありません".to_string());
        return Ok(());
    }
    let Some(card) = store.race_card(&race_id).await? else {
        fail("出馬表がありません".to_string());
        return Ok(());
    };
    let existing = store.existing(&race_id).await?;
    let plan = match plan_race(&card, &existing, &page) {
        Ok(p) => p,
        Err(mismatches) => {
            let why = mismatches
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" / ");
            fail(format!("照合に失敗: {why}"));
            return Ok(());
        }
    };
    let planned = plan.missing_nums();
    let inserted = if dry_run {
        None
    } else if plan.missing.is_empty() {
        Some(Vec::new())
    } else {
        let got = store.insert(&card, &plan.missing).await?;
        if got != planned {
            report.failures.push((
                t.race_id.clone(),
                format!("INSERT は済んだが、入った馬番 {got:?} が計画 {planned:?} と違います"),
            ));
        }
        Some(got)
    };
    report.races.push(RaceReport {
        race_id: t.race_id.clone(),
        existing: plan.existing,
        page_runners: plan.page_runners,
        planned,
        inserted,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    use paddock_domain::ResultStatus;

    use crate::fetch::tests::result_page;
    use crate::plan::tests::fixtures::{card, card_for, db};

    #[derive(Default)]
    struct FakeStore {
        cards: HashMap<String, RaceCard>,
        existing: HashMap<String, Vec<ExistingResultRow>>,
        /// races 行が無いレース（既定は全レースにある）。
        no_races_row: HashSet<String>,
        /// 既存行の読み出しで DB エラーにするレース。
        broken: HashSet<String>,
        /// INSERT が返す馬番を差し替える（同時に別経路で入った場合などを模す）。
        insert_returns: Option<Vec<u32>>,
        inserts: RefCell<Vec<(String, Vec<u32>)>>,
    }

    impl FillStore for FakeStore {
        async fn race_exists(&self, race_id: &RaceId) -> anyhow::Result<bool> {
            Ok(!self.no_races_row.contains(race_id.value()))
        }
        async fn race_card(&self, race_id: &RaceId) -> anyhow::Result<Option<RaceCard>> {
            Ok(self.cards.get(race_id.value()).cloned())
        }
        async fn existing(&self, race_id: &RaceId) -> anyhow::Result<Vec<ExistingResultRow>> {
            anyhow::ensure!(!self.broken.contains(race_id.value()), "connection reset");
            Ok(self
                .existing
                .get(race_id.value())
                .cloned()
                .unwrap_or_default())
        }
        async fn insert(&self, card: &RaceCard, rows: &[ResultRow]) -> anyhow::Result<Vec<u32>> {
            let nums: Vec<u32> = rows.iter().map(|r| r.horse_num.value()).collect();
            self.inserts
                .borrow_mut()
                .push((card.race_id.value().to_string(), nums.clone()));
            Ok(self.insert_returns.clone().unwrap_or(nums))
        }
    }

    const RID: &str = "2026-2-niigata-6-7R";

    fn target() -> Target {
        Target {
            race_id: RID.into(),
            netkeiba_id: "202604020607".into(),
        }
    }

    fn setup(dir: &Path, page: &[(&str, u32)], existing: Vec<ExistingResultRow>) -> FakeStore {
        std::fs::create_dir_all(dir.join("raw")).unwrap();
        std::fs::write(html_path(dir, RID), result_page(page)).unwrap();
        FakeStore {
            cards: HashMap::from([(RID.to_string(), card(&[1, 2, 3]))]),
            existing: HashMap::from([(RID.to_string(), existing)]),
            ..FakeStore::default()
        }
    }

    #[tokio::test]
    async fn inserts_only_the_planned_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = setup(
            dir.path(),
            &[("1", 1), ("2", 2), ("除", 3)],
            vec![db(1, ResultStatus::Finished)],
        );
        let report = run(&store, &[target()], dir.path(), false).await;
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(*store.inserts.borrow(), vec![(RID.to_string(), vec![2, 3])]);
        assert_eq!(
            report.races,
            vec![RaceReport {
                race_id: RID.into(),
                existing: 1,
                page_runners: 2,
                planned: vec![2, 3],
                inserted: Some(vec![2, 3]),
            }]
        );
    }

    #[tokio::test]
    async fn dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = setup(
            dir.path(),
            &[("1", 1), ("2", 2)],
            vec![db(1, ResultStatus::Finished)],
        );
        let report = run(&store, &[target()], dir.path(), true).await;
        assert!(store.inserts.borrow().is_empty());
        assert_eq!(report.races[0].planned, vec![2]);
        assert_eq!(report.races[0].inserted, None);
    }

    #[tokio::test]
    async fn mismatch_writes_nothing_and_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        // DB では 1 番が除外なのに、結果ページでは 1 着
        let store = setup(
            dir.path(),
            &[("1", 1), ("2", 2)],
            vec![db(1, ResultStatus::Scratched)],
        );
        let report = run(&store, &[target()], dir.path(), false).await;
        assert!(store.inserts.borrow().is_empty());
        assert!(report.races.is_empty());
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].1.contains("照合に失敗"),
            "{:?}",
            report.failures
        );
    }

    #[tokio::test]
    async fn missing_html_or_card_is_a_failure_and_other_races_continue() {
        let dir = tempfile::tempdir().unwrap();
        let store = setup(
            dir.path(),
            &[("1", 1), ("2", 2)],
            vec![db(1, ResultStatus::Finished)],
        );
        let no_html = Target {
            race_id: "2026-2-niigata-6-8R".into(),
            netkeiba_id: "202604020608".into(),
        };
        // HTML はあるが出馬表が無いレース
        let no_card = Target {
            race_id: "2026-2-niigata-6-9R".into(),
            netkeiba_id: "202604020609".into(),
        };
        std::fs::write(
            html_path(dir.path(), &no_card.race_id),
            result_page(&[("1", 1)]),
        )
        .unwrap();
        let report = run(&store, &[no_html, no_card, target()], dir.path(), false).await;
        let why: Vec<&str> = report.failures.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(why.len(), 2, "{why:?}");
        assert!(why[0].contains("HTML"), "{why:?}");
        assert!(why[1].contains("出馬表"), "{why:?}");
        assert_eq!(*store.inserts.borrow(), vec![(RID.to_string(), vec![2])]);
    }

    #[tokio::test]
    async fn missing_races_row_is_a_failure_even_in_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = setup(
            dir.path(),
            &[("1", 1), ("2", 2)],
            vec![db(1, ResultStatus::Finished)],
        );
        store.no_races_row.insert(RID.to_string());
        for dry_run in [true, false] {
            let report = run(&store, &[target()], dir.path(), dry_run).await;
            assert!(report.races.is_empty(), "dry_run={dry_run}");
            assert_eq!(report.failures.len(), 1);
            assert!(
                report.failures[0].1.contains("races 行"),
                "{:?}",
                report.failures
            );
        }
        assert!(store.inserts.borrow().is_empty());
    }

    #[tokio::test]
    async fn db_error_stops_but_keeps_what_was_done_before() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = setup(
            dir.path(),
            &[("1", 1), ("2", 2)],
            vec![db(1, ResultStatus::Finished)],
        );
        let broken = Target {
            race_id: "2026-2-niigata-6-8R".into(),
            netkeiba_id: "202604020608".into(),
        };
        let after = Target {
            race_id: "2026-2-niigata-6-9R".into(),
            netkeiba_id: "202604020609".into(),
        };
        for t in [&broken, &after] {
            std::fs::write(html_path(dir.path(), &t.race_id), result_page(&[("1", 1)])).unwrap();
            store
                .cards
                .insert(t.race_id.clone(), card_for(&t.race_id, &[1]));
        }
        store.broken.insert(broken.race_id.clone());
        let report = run(&store, &[target(), broken, after], dir.path(), false).await;
        // 1 本目は書いて報告に残る。2 本目の DB エラーで止まり、3 本目は触らない
        assert_eq!(report.races.len(), 1);
        assert_eq!(report.races[0].inserted, Some(vec![2]));
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].1.contains("止めました"),
            "{:?}",
            report.failures
        );
        assert_eq!(*store.inserts.borrow(), vec![(RID.to_string(), vec![2])]);
    }

    #[tokio::test]
    async fn unexpected_insert_is_reported_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = setup(
            dir.path(),
            &[("1", 1), ("2", 2), ("3", 3)],
            vec![db(1, ResultStatus::Finished)],
        );
        store.insert_returns = Some(vec![2]);
        let report = run(&store, &[target()], dir.path(), false).await;
        assert_eq!(report.races[0].inserted, Some(vec![2]));
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].1.contains("INSERT は済んだ"),
            "{:?}",
            report.failures
        );
    }

    #[test]
    fn db_name_must_match() {
        assert!(check_db_name("paddock_test742", "paddock_test742").is_ok());
        let err = check_db_name("paddock", "paddock_test742").unwrap_err();
        assert!(err.to_string().contains("書きません"), "{err}");
    }
}
