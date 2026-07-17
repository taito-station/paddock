use serde::Serialize;
use utoipa::ToSchema;

use paddock_use_case::RefreshReport;

/// `POST /api/results/{date}:refresh`（およびエイリアス `.../sessions/{date}/results:refresh`）の
/// レスポンス（#381）。同日結果取り込み＋自動精算の結果。精算集計（`settled_races`〜`roi`）は
/// 従来の `SettleReportResponse` と同一で、着順取り込みの確定レース情報を加える（レスポンス上位互換）。
#[derive(Debug, Serialize, ToSchema)]
pub struct RefreshReportResponse {
    pub settled_races: u32,
    pub pending_races: u32,
    pub voided_races: u32,
    pub refunded_bets: u32,
    pub total_bet: u64,
    pub total_payout: u64,
    pub balance: u64,
    /// 回収率(%)。総賭け金 0 なら null。
    pub roi: Option<f64>,
    /// 当パスで新規に着順を取り込んで確定したレース数（#381）。
    pub newly_confirmed_races: u32,
    /// 当パスで新規に確定したレースの `race_id`（昇順）。
    pub confirmed_race_ids: Vec<String>,
}

impl From<RefreshReport> for RefreshReportResponse {
    fn from(r: RefreshReport) -> Self {
        Self {
            settled_races: r.settled_races as u32,
            pending_races: r.pending_races as u32,
            voided_races: r.voided_races as u32,
            refunded_bets: r.refunded_bets as u32,
            total_bet: r.total_bet,
            total_payout: r.total_payout,
            balance: r.balance,
            roi: r.roi,
            newly_confirmed_races: r.newly_confirmed_races as u32,
            confirmed_race_ids: r
                .confirmed_race_ids
                .iter()
                .map(|id| id.value().to_string())
                .collect(),
        }
    }
}
