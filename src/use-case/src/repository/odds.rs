use core::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use paddock_domain::{BetType, OrderedPair, OrderedTriple, Pair, RaceId, RaceOdds, Triple};

use crate::error::Result;

/// `race_odds` テーブルへ保存する 1 行分のオッズ。ドメインの [`RaceOdds`] は
/// popularity / fetched_at を持てないため、永続化専用の入力型として use-case 層に定義する。
#[derive(Debug, Clone)]
pub struct OddsRow {
    /// 馬券種ラベル（単勝なら `"win"`）。
    pub bet_type: String,
    /// 組み合わせキー（単勝なら馬番文字列）。
    pub combination_key: String,
    pub odds: f64,
    /// 複勝のような下限/上限を持つ馬券種の上限値。単勝は `None`。
    pub odds_high: Option<f64>,
    pub popularity: Option<u32>,
}

impl OddsRow {
    /// 単勝 1 行。`combination_key` は素の馬番文字列（"1".."18"、ゼロ詰めしない）。
    pub fn win(horse_num: u32, odds: f64, popularity: Option<u32>) -> Self {
        Self {
            bet_type: BetType::Win.to_string(),
            combination_key: horse_num.to_string(),
            odds,
            odds_high: None,
            popularity,
        }
    }

    /// 複勝 1 行。幅 odds を `odds`=下限・`odds_high`=上限 に詰める。
    /// `combination_key` は素の馬番文字列（単勝と同じ規約）。
    pub fn place(horse_num: u32, low: f64, high: f64, popularity: Option<u32>) -> Self {
        Self {
            bet_type: BetType::Place.to_string(),
            combination_key: horse_num.to_string(),
            odds: low,
            odds_high: Some(high),
            popularity,
        }
    }

    // 組合せ券種(#38)はライブスクレイプ由来で人気を持たないため popularity は None 固定。

    /// 馬連 1 行。キーは昇順 `Pair`（`"1-2"`）。
    pub fn quinella(pair: Pair, odds: f64) -> Self {
        Self {
            bet_type: BetType::Quinella.to_string(),
            combination_key: pair.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }

    /// ワイド 1 行。複勝と同じく幅 odds（`odds`=下限・`odds_high`=上限）。キーは昇順 `Pair`。
    pub fn wide(pair: Pair, low: f64, high: f64) -> Self {
        Self {
            bet_type: BetType::Wide.to_string(),
            combination_key: pair.to_key(),
            odds: low,
            odds_high: Some(high),
            popularity: None,
        }
    }

    /// 馬単 1 行。キーは順序付き `OrderedPair`（`"1>2"`）。
    pub fn exacta(pair: OrderedPair, odds: f64) -> Self {
        Self {
            bet_type: BetType::Exacta.to_string(),
            combination_key: pair.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }

    /// 三連複 1 行。キーは昇順 `Triple`（`"1-2-3"`）。
    pub fn trio(triple: Triple, odds: f64) -> Self {
        Self {
            bet_type: BetType::Trio.to_string(),
            combination_key: triple.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }

    /// 三連単 1 行。キーは順序付き `OrderedTriple`（`"1>2>3"`）。
    pub fn trifecta(triple: OrderedTriple, odds: f64) -> Self {
        Self {
            bet_type: BetType::Trifecta.to_string(),
            combination_key: triple.to_key(),
            odds,
            odds_high: None,
            popularity: None,
        }
    }
}

/// 1 レース分のオッズ取得結果。取得時刻 `fetched_at` は use-case 層で注入し、
/// gateway を時計から独立に保つ（[`crate::repository::FetchRecord`] と同じ流儀）。
#[derive(Debug, Clone)]
pub struct RaceOddsRecord {
    pub race_id: RaceId,
    pub fetched_at: DateTime<Utc>,
    pub rows: Vec<OddsRow>,
}

/// 朝時点オッズと、その取得時刻・最新取得時刻（#448 朝↔現比較）。
///
/// `odds` は `race_odds_snapshots` のうち**最初にフル盤（買い目が組める完全なオッズ）が成立した**
/// スナップショットを再構成したもの（＝最小 `fetched_at` そのものではない。早朝の単複のみスイープは
/// exotic 欠落で ROI が組めず除外する。詳細は [`OddsRepository::find_race_odds_morning`]）。
/// `morning_at`/`latest_at` は UTC(RFC3339) 文字列（表示は呼び出し側で JST 整形）。朝 complete が無い、
/// または朝 complete==最新で比較の意味が無い場合はそもそも `Some` を返さない運用。
#[derive(Debug, Clone)]
pub struct MorningRaceOdds {
    pub odds: RaceOdds,
    /// 朝時点（最初にフル盤成立したスナップショット）の取得時刻。
    pub morning_at: String,
    /// 最新スイープ（＝現時点）の取得時刻。
    pub latest_at: String,
}

/// レースオッズ（`race_odds`）の保存・取得。
pub trait OddsRepository: Send + Sync {
    /// 1 レース分のオッズ（行単位）を upsert する。`race_odds` の主キー
    /// `(race_id, bet_type, combination_key)` で衝突した行は最新値で更新する。
    fn save_race_odds(&self, record: &RaceOddsRecord) -> impl Future<Output = Result<()>> + Send;

    /// `race_odds` に保存済みのオッズを全券種読み出してドメインの [`RaceOdds`] に再構成する。
    /// `as_of = Some(d)` のとき `date(fetched_at) <= d` のスナップショットのみ対象とする
    /// （backtest の当時オッズ参照用、リーク防止）。`None` は時刻制約なし（predict の最新参照用）。
    /// いずれの券種の行も無ければ `None` を返す。
    fn find_race_odds(
        &self,
        race_id: &RaceId,
        as_of: Option<NaiveDate>,
    ) -> impl Future<Output = Result<Option<RaceOdds>>> + Send;

    /// 朝時点オッズを `race_odds_snapshots` から復元する（#448）。朝時点＝最初にフル盤（買い目が
    /// 組める完全なオッズ）が成立したスナップショット（最小 `fetched_at` ではない。実装で
    /// [`RaceOdds::is_complete`] を満たす最古時刻を採る）。盤の「朝↔現比較」用。朝 complete が無い
    /// （単複のみ履歴＝現時点も ROI を出せない）、または朝 complete==最新（比較する差が無い）なら `None`。
    fn find_race_odds_morning(
        &self,
        race_id: &RaceId,
    ) -> impl Future<Output = Result<Option<MorningRaceOdds>>> + Send;

    /// `race_odds_snapshots`（append-only 履歴, #232）のうち `fetched_at` の日付が `before`
    /// より前の行を削除し、削除行数を返す（retention/パージ, #234）。最新キャッシュ `race_odds` は
    /// 対象外。`before` 当日（`date(fetched_at) == before`）は残す（厳密 `<`）。
    fn purge_race_odds_snapshots(
        &self,
        before: NaiveDate,
    ) -> impl Future<Output = Result<u64>> + Send;

    /// `purge_race_odds_snapshots` の削除対象行数を、削除せずに数える（dry-run 用, #234）。
    fn count_race_odds_snapshots_before(
        &self,
        before: NaiveDate,
    ) -> impl Future<Output = Result<u64>> + Send;
}
