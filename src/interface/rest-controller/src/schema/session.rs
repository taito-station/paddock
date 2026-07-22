use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use paddock_domain::RaceId;
use paddock_use_case::repository::{PredictBetRecord, PredictSessionRecord};

/// `POST /api/sessions/{date}` のリクエスト。
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// 初期予算（円）。1 以上。
    #[schema(minimum = 1)]
    pub budget: u64,
}

/// outcome 記録の 1 買い目（リクエスト）。race_id はパスから取るため含めない。
#[derive(Debug, Deserialize, ToSchema)]
pub struct BetInput {
    /// 馬券種ラベル（例 `単勝` / `馬連`）。
    pub bet_type: String,
    /// 組み合わせコード（例 `7` / `7-14`）。
    pub combination: String,
    /// 賭け金（円）。
    pub stake: u64,
    /// 払戻（円、記録時点で不明なら 0。results:refresh で確定値に上書きされる）。
    #[serde(default)]
    pub payout: u64,
    /// 期待値（参考値）。
    #[serde(default)]
    pub ev: f64,
}

/// `POST /api/sessions/{date}/races/{race_id}/outcome` のリクエスト。
#[derive(Debug, Deserialize, ToSchema)]
pub struct RecordOutcomeRequest {
    /// このレースで購入した買い目（空 = スキップ相当）。
    pub bets: Vec<BetInput>,
}

/// サマリの 1 買い目（レスポンス）。どのレースの買い目かを race_id で示す。
#[derive(Debug, Serialize, ToSchema)]
pub struct SummaryBet {
    pub race_id: String,
    pub bet_type: String,
    pub combination: String,
    pub stake: u64,
    pub payout: u64,
    pub ev: f64,
}

impl From<&PredictBetRecord> for SummaryBet {
    fn from(b: &PredictBetRecord) -> Self {
        Self {
            race_id: b.race_id.value().to_string(),
            bet_type: b.bet_type.clone(),
            combination: b.combination.clone(),
            stake: b.stake,
            payout: b.payout,
            ev: b.ev,
        }
    }
}

/// セッション収支サマリ（作成 / outcome / GET summary 共通のレスポンス）。
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionSummaryResponse {
    pub date: NaiveDate,
    pub budget: u64,
    pub balance: u64,
    pub total_bet: u64,
    pub total_payout: u64,
    /// 損益（`total_payout − total_bet`。負もあるため i64）。
    pub pnl: i64,
    pub completed: bool,
    pub bets: Vec<SummaryBet>,
    /// 「見送り（スキップ）」として記録済みのレース ID（#481）。買い目ありで記録した
    /// レースは `bets` 側に現れるためここには含まれない。web 盤が再訪時に「見送り済み」
    /// バッジを出す判定に使う。
    pub skipped_race_ids: Vec<String>,
}

impl SessionSummaryResponse {
    pub fn new(
        session: &PredictSessionRecord,
        bets: &[PredictBetRecord],
        skipped: &[RaceId],
    ) -> Self {
        Self {
            date: session.date,
            budget: session.budget,
            balance: session.balance,
            total_bet: session.total_bet,
            total_payout: session.total_payout,
            pnl: session.total_payout as i64 - session.total_bet as i64,
            completed: session.completed,
            bets: bets.iter().map(SummaryBet::from).collect(),
            skipped_race_ids: skipped.iter().map(|r| r.value().to_string()).collect(),
        }
    }
}

/// `POST .../odds:refresh` のレスポンス。
#[derive(Debug, Serialize, ToSchema)]
pub struct OddsRefreshResponse {
    pub race_id: String,
    /// オッズを取得できたか（false = 未公開・取得失敗で未取得。HTTP は 200）。
    pub fetched: bool,
}
