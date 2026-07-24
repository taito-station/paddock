use core::future::Future;
use std::collections::HashMap;

use chrono::NaiveDate;
use paddock_domain::{
    HorseName, JockeyFormRun, JockeyName, Mark, Race, RecentRun, StandardTimes, Surface,
    TrainerName, Venue,
};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct GroupStat {
    pub label: String,
    pub starts: u32,
    pub wins: u32,
    pub places: u32,
    pub shows: u32,
}

impl GroupStat {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            starts: 0,
            wins: 0,
            places: 0,
            shows: 0,
        }
    }

    pub fn win_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.wins as f64 / self.starts as f64
        }
    }

    pub fn place_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.places as f64 / self.starts as f64
        }
    }

    pub fn show_rate(&self) -> f64 {
        if self.starts == 0 {
            0.0
        } else {
            self.shows as f64 / self.starts as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct HorseStatsRow {
    pub horse_name: String,
    pub by_surface: Vec<GroupStat>,
    pub by_distance_band: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
    pub by_track_condition: Vec<GroupStat>,
    pub by_popularity_band: Vec<GroupStat>,
    /// 競馬場（venue）別成績（#350 measure-first）。ラベルは日本語場名（`races.venue` の値＝
    /// `Venue::as_jp()` と一致）。当該馬が走ったことのある場のみ含む（動的キー GROUP BY）。
    pub by_venue: Vec<GroupStat>,
    /// 騎手別成績（#350 measure-first、騎手×馬コンビ用）。ラベルは騎手名（`results.jockey` の値）。
    /// この馬に騎乗したことのある騎手のみ含む（動的キー GROUP BY）。build_factors が現騎手名で引く。
    pub by_jockey: Vec<GroupStat>,
    pub overall: GroupStat,
}

#[derive(Debug, Clone)]
pub struct CourseStatsRow {
    pub venue: String,
    pub distance: u32,
    pub surface: String,
    pub by_gate_group: Vec<GroupStat>,
}

/// 条件依存枠バイアスの頭数帯 `(ラベル, 下限, 上限)`。集計（rdb-gateway の SQL）と提示側の当日頭数
/// 分類（[`gate_field_band_label`]）が共有する**単一の真実源**（ラベル drift 防止, #343）。
/// 多帯の上限 99 はラベル表記（14-18）より広いが、JRA 実頭数上限は 18 なので実害はなく、18 超の
/// 異常データを多帯に吸収するための余裕（BETWEEN の上端）。
pub const GATE_FIELD_BANDS: &[(&str, u32, u32)] = &[
    ("多(14-18)", 14, 99),
    ("中(10-13)", 10, 13),
    ("少(-9)", 1, 9),
];

/// 馬場 2 値ラベル: 良。
pub const GATE_TRACK_FIRM: &str = "良";
/// 馬場 2 値ラベル: 非良（稍重・重・不良）。
pub const GATE_TRACK_OTHER: &str = "非良";

/// 当日の出走頭数を [`GATE_FIELD_BANDS`] の帯ラベルへ写す（集計セルと同じ区分, #343）。
pub fn gate_field_band_label(field_size: u32) -> &'static str {
    GATE_FIELD_BANDS
        .iter()
        .find(|(_, lo, hi)| field_size >= *lo && field_size <= *hi)
        .map(|(label, _, _)| *label)
        // 全帯（1..=99）を覆うので通常到達しないが、範囲外は最少帯に寄せる。
        .unwrap_or(GATE_FIELD_BANDS[GATE_FIELD_BANDS.len() - 1].0)
}

/// 馬場状態文字列を 良/非良 の 2 値ラベルへ写す（[`GATE_TRACK_FIRM`]/[`GATE_TRACK_OTHER`], #343）。
pub fn gate_track_cond2_label(track_condition: &str) -> &'static str {
    if track_condition == GATE_TRACK_FIRM {
        GATE_TRACK_FIRM
    } else {
        GATE_TRACK_OTHER
    }
}

/// 条件依存の枠バイアス集計 1 セル（馬場2値 × 頭数帯 × 枠群）の複勝ベース率（#343・提示専用）。
#[derive(Debug, Clone)]
pub struct GateBiasCell {
    /// 馬場ラベル（"良" / "非良"）。
    pub track_label: String,
    /// 頭数帯ラベル（"多(14-18)" / "中(10-13)" / "少(-9)"）。
    pub field_label: String,
    /// 枠群ラベル（`GATE_GROUPS` と同一: "Inner (1-3)" 等）。
    pub gate_label: String,
    pub stat: GroupStat,
}

/// コース（場×距離×馬場）の「馬場状態 × 頭数帯 × 枠群」条件依存枠バイアス集計（#343）。
/// **提示専用**（decision-support）でスコアには入れない（measure-first）。薄いセルは呼び出し側が
/// `stat.starts` で信頼度を判断する。既知の枠バイアスは市場が織り込み済みのため、edge は「市場が
/// 雑にしか評価していない交互作用」だけに宿る想定（本 issue で市場差分により可視化する）。
#[derive(Debug, Clone)]
pub struct ConditionalGateStatsRow {
    pub cells: Vec<GateBiasCell>,
}

impl ConditionalGateStatsRow {
    /// 指定セル（馬場×頭数×枠）を引く。該当なしは `None`。
    pub fn cell(
        &self,
        track_label: &str,
        field_label: &str,
        gate_label: &str,
    ) -> Option<&GateBiasCell> {
        self.cells.iter().find(|c| {
            c.track_label == track_label
                && c.field_label == field_label
                && c.gate_label == gate_label
        })
    }

    /// 同条件（馬場×頭数）の全枠合算の複勝率＝枠効果 lift の基準線。starts 合計 0 なら `None`。
    pub fn condition_show_rate(&self, track_label: &str, field_label: &str) -> Option<f64> {
        let (mut starts, mut shows) = (0u32, 0u32);
        for c in self
            .cells
            .iter()
            .filter(|c| c.track_label == track_label && c.field_label == field_label)
        {
            starts += c.stat.starts;
            shows += c.stat.shows;
        }
        (starts > 0).then(|| shows as f64 / starts as f64)
    }
}

/// recency 重み付け（#75 Phase B）用に、あるカテゴリの 1 ラベルぶんの日付付き成績系列。
/// `runs` は `races.date < as_of` の各開催日のカウント（同一日複数走は 1 要素にまとめる）。
#[derive(Debug, Clone, Default)]
pub struct RecencySeries {
    pub label: String,
    pub runs: Vec<paddock_domain::DatedCounts>,
}

/// 馬の成績を recency 重み付け用に「カテゴリ × ラベル別の日付付き系列」で返す（#75 Phase B）。
/// 集計済み [`HorseStatsRow`] と違い各開催日のカウントを保持し、domain 側で時間減衰を掛ける。
/// recency 無効時は取得しない（mock・既定実装は空）。
#[derive(Debug, Clone, Default)]
pub struct HorseRecencyStats {
    pub by_surface: Vec<RecencySeries>,
    pub by_distance_band: Vec<RecencySeries>,
    pub by_track_condition: Vec<RecencySeries>,
}

#[derive(Debug, Clone)]
pub struct JockeyStatsRow {
    pub jockey_name: String,
    pub overall: GroupStat,
    pub by_surface: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
    /// 競馬場（venue）別成績（#350 measure-first）。ラベルは日本語場名（`races.venue` の値＝
    /// `Venue::as_jp()` と一致）。当該騎手が騎乗したことのある場のみ含む（動的キー GROUP BY）。
    pub by_venue: Vec<GroupStat>,
    /// 距離帯別成績（#350 measure-first）。ラベルは horse の距離帯（〜1400m 等）と同一区分。
    pub by_distance_band: Vec<GroupStat>,
}

#[derive(Debug, Clone)]
pub struct TrainerStatsRow {
    pub trainer_name: String,
    pub overall: GroupStat,
    pub by_surface: Vec<GroupStat>,
    pub by_gate_group: Vec<GroupStat>,
}

/// 印別集計（#145）の母集団フィルタ。母集団は結果記録済み（`finish_1 IS NOT NULL`）の予想に限る。
#[derive(Debug, Clone, Default)]
pub struct MarkStatsFilter {
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    pub venue: Option<Venue>,
}

/// 印 1 種の的中率集計。`count` はその印が付いた（結果記録済みの）馬の延べ数。
/// `win` = 1 着、`show` = 複勝圏（3 着内）に入った延べ数。
#[derive(Debug, Clone)]
pub struct MarkStatRow {
    pub mark: Mark,
    pub count: u32,
    pub win: u32,
    pub show: u32,
}

impl MarkStatRow {
    pub fn win_rate(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.win as f64 / self.count as f64
        }
    }

    pub fn show_rate(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.show as f64 / self.count as f64
        }
    }
}

/// 馬・騎手・調教師・コースの成績統計と、標準タイム・近走・確定レース系の読み出し。
pub trait StatsRepository: Send + Sync {
    /// 馬の各種成績統計を返す。`as_of = Some(d)` のとき `races.date < d` の成績のみを集計する
    /// （バックテストのリーク防止。本番予想は `None` で全期間集計）。
    fn horse_stats(
        &self,
        name: &HorseName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HorseStatsRow>> + Send;

    /// 複数馬の `horse_stats` をまとめて返す（#196 backtest の N+1 解消）。`names` の各馬を
    /// キーに [`HorseStatsRow`] を引く。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    /// 既定実装は per-item `horse_stats` をループするだけで挙動は変わらない（mock・predict 経路は
    /// この既定で十分。rdb-gateway のみが 1 レース一括クエリで override する）。返却 map は `names`
    /// に現れた全馬のエントリを含む（重複名は 1 回だけ引く）。
    fn horse_stats_batch(
        &self,
        names: &[HorseName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<HorseName, HorseStatsRow>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.horse_stats(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// recency 重み付け（#75 Phase B）用に、馬の成績を「カテゴリ × ラベル別の日付付き系列」で返す。
    /// `as_of` の意味は [`StatsRepository::horse_stats`] と同じ（`races.date < as_of`）。既定実装は空を返す
    /// （recency 無効時の本番経路・テスト mock はこの既定で十分。日付付き集計が要るのは
    /// rdb-gateway のみがオーバーライドする）。
    fn horse_recency(
        &self,
        _name: &HorseName,
        _as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HorseRecencyStats>> + Send {
        async { Ok(HorseRecencyStats::default()) }
    }

    /// 複数馬の `horse_recency` をまとめて返す（#196）。既定実装は per-item `horse_recency` を
    /// ループするだけで挙動は変わらない（既定 `horse_recency` は空を返すため、recency 無効時の
    /// 本番経路・mock では空 map ではなく全馬の空系列が入る点に注意）。rdb-gateway のみが
    /// 1 レース一括クエリで override する。返却 map は `names` の全馬を含む。
    /// なお、この既定実装が返す「全馬の空系列」を実際に使うのは default 実装を踏む経路（mock 等）
    /// だけで、backtest は recency 無効時にそもそも本メソッドを呼ばない（呼ぶのは Postgres override のみ）。
    fn horse_recency_batch(
        &self,
        names: &[HorseName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<HorseName, HorseRecencyStats>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.horse_recency(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// コース（場×距離×馬場）の枠順別統計を返す。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    fn course_stats(
        &self,
        venue: Venue,
        distance: u32,
        surface: Surface,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<CourseStatsRow>> + Send;

    /// コースの「馬場状態 × 頭数帯 × 枠群」条件依存枠バイアス（複勝ベース率）を返す（#343・提示専用）。
    /// 既定は空（mock・提示不要経路。実集計は rdb-gateway のみ override）。`as_of` は他 stats と同義。
    fn conditional_gate_stats(
        &self,
        _venue: Venue,
        _distance: u32,
        _surface: Surface,
        _as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<ConditionalGateStatsRow>> + Send {
        async move { Ok(ConditionalGateStatsRow { cells: Vec::new() }) }
    }

    /// 騎手の各種成績統計を返す。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    fn jockey_stats(
        &self,
        name: &JockeyName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<JockeyStatsRow>> + Send;

    /// 複数騎手の `jockey_stats` をまとめて返す（#196）。既定実装は per-item をループするだけ。
    /// rdb-gateway のみが 1 レース一括クエリで override する。返却 map は `names` の全騎手を含む。
    fn jockey_stats_batch(
        &self,
        names: &[JockeyName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<JockeyName, JockeyStatsRow>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.jockey_stats(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// 調教師の各種成績統計を返す。`as_of` の意味は [`StatsRepository::horse_stats`] と同じ。
    fn trainer_stats(
        &self,
        name: &TrainerName,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<TrainerStatsRow>> + Send;

    /// 複数調教師の `trainer_stats` をまとめて返す（#196）。既定実装は per-item をループするだけ。
    /// rdb-gateway のみが 1 レース一括クエリで override する。返却 map は `names` の全調教師を含む。
    fn trainer_stats_batch(
        &self,
        names: &[TrainerName],
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<HashMap<TrainerName, TrainerStatsRow>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(name.clone(), self.trainer_stats(name, as_of).await?);
            }
            Ok(out)
        }
    }

    /// 指定期間 `[from, to]`（両端含む）の確定済みレースを `results` 付きで race_num 昇順に返す。
    /// `races.source='pdf'` かつ着順ありの `results` を 1 件以上含むレースのみを対象とする
    /// （バックテストの評価対象取得用）。`from > to` のときは空 Vec を返す。
    ///
    /// 命名は race 寄りだが、backtest の評価対象を集計読み出しする用途のため `StatsRepository` に置く
    /// （backtest interactor の束縛を `StatsRepository + OddsRepository` に閉じ、mock を最小化するため）。
    fn find_finished_races_between(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> impl Future<Output = Result<Vec<Race>>> + Send;

    /// 指定馬の `before` より前（`races.date < before`）の成績を date 降順で最大 `limit` 件返す。
    /// 各要素は `RecentRun`（開催日・当該レースの surface/distance・成績）。前走フォーム特徴量
    /// （#31/#76）の算出に使う。surface/distance は前走タイムを標準タイムへ突き合わせるために運ぶ。
    /// `before` 制約によりバックテスト時のリークを防ぐ。pdf/netkeiba 双方の成績を対象とする（実際の前走）。
    fn find_recent_runs(
        &self,
        name: &HorseName,
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<RecentRun>>> + Send;

    /// 複数馬の `find_recent_runs` をまとめて返す（#196）。各馬につき `before` より前の直近 `limit`
    /// 件を date 降順で引く。既定実装は per-item `find_recent_runs` をループするだけで挙動は変わらない
    /// （mock・predict 経路はこの既定で十分）。rdb-gateway のみが全馬一括の window 関数で override
    /// する。返却 map は `names` の全馬を含む（前走が無い馬は空 `Vec`）。
    fn recent_runs_batch(
        &self,
        names: &[HorseName],
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<HashMap<HorseName, Vec<RecentRun>>>> + Send {
        async move {
            let mut out = HashMap::new();
            for name in names {
                if out.contains_key(name) {
                    continue;
                }
                out.insert(
                    name.clone(),
                    self.find_recent_runs(name, before, limit).await?,
                );
            }
            Ok(out)
        }
    }

    /// 指定騎手の `before` より前（`races.date < before`）の近走を date 降順で最大 `limit` 件返す
    /// （#221）。戻り要素 `JockeyFormRun` は着順・人気のみを運ぶ（フォームシグナル算出に特化）。
    /// `before` 制約によりバックテスト時のリークを防ぐ。pdf/netkeiba 双方の成績を対象とする。
    fn find_jockey_recent_runs(
        &self,
        jockey: &JockeyName,
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<JockeyFormRun>>> + Send;

    /// 複数騎手の `find_jockey_recent_runs` をまとめて返す（#221）。既定実装は per-item ループ。
    /// rdb-gateway のみが全騎手一括クエリで override する。返却 map は `jockeys` の全騎手を含む
    /// （近走が無い騎手は空 `Vec`）。
    fn jockey_recent_runs_batch(
        &self,
        jockeys: &[JockeyName],
        before: NaiveDate,
        limit: u32,
    ) -> impl Future<Output = Result<HashMap<JockeyName, Vec<JockeyFormRun>>>> + Send {
        async move {
            let mut out = HashMap::new();
            for j in jockeys {
                if out.contains_key(j) {
                    continue;
                }
                out.insert(
                    j.clone(),
                    self.find_jockey_recent_runs(j, before, limit).await?,
                );
            }
            Ok(out)
        }
    }

    /// `before` より前（`races.date < before`）のコーパスから (surface, distance) 別の標準タイム
    /// （代表タイム[秒]）を集計して返す（#76）。前走タイムを相対速度シグナルへ変換する分母に使う。
    /// `before` 制約で as-of リークを防ぐ。標本数が閾値未満の薄いバケツは含めない。
    fn standard_times(
        &self,
        before: NaiveDate,
    ) -> impl Future<Output = Result<StandardTimes>> + Send;
}

/// `analyze` の部分一致候補（馬名・騎手名・調教師名）の検索。
pub trait NameMatchRepository: Send + Sync {
    /// `analyze` の部分一致検索用。`results` に `query` を中間一致（`LIKE '%query%'`）する
    /// 馬名を重複排除して名前昇順で最大 `limit` 件返す。`query` は呼び出し側で正規化済みとする。
    fn find_matching_horse_names(
        &self,
        query: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// 騎手名版（[`NameMatchRepository::find_matching_horse_names`] と同方針）。
    fn find_matching_jockey_names(
        &self,
        query: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// 調教師名版（[`NameMatchRepository::find_matching_horse_names`] と同方針）。
    fn find_matching_trainer_names(
        &self,
        query: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;
}
