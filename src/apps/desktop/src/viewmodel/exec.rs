use paddock_domain::portfolio::{Portfolio, PortfolioBet};
use paddock_use_case::repository::session::PredictSessionRecord;

#[derive(Debug, Clone, PartialEq)]
pub struct BetView {
    pub bet_type: String,
    pub combination: String,
    pub stake: u64,
    pub ev: String,
    pub hit_prob: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    pub budget: u64,
    pub balance: u64,
    pub total_bet: u64,
    pub completed: bool,
}

impl From<&PortfolioBet> for BetView {
    fn from(b: &PortfolioBet) -> Self {
        Self {
            bet_type: bet_type_jp(b.combination.type_label()),
            combination: b.combination.combination_code(),
            stake: b.stake,
            ev: format!("{:.2}", b.ev),
            hit_prob: format!("{:.1}%", b.hit_prob * 100.0),
        }
    }
}

impl From<&PredictSessionRecord> for SessionView {
    fn from(s: &PredictSessionRecord) -> Self {
        Self {
            budget: s.budget,
            balance: s.balance,
            total_bet: s.total_bet,
            completed: s.completed,
        }
    }
}

pub fn portfolio_to_bets(p: &Portfolio) -> Vec<BetView> {
    p.bets.iter().map(BetView::from).collect()
}

fn bet_type_jp(slug: &str) -> String {
    match slug {
        "win" => "単勝",
        "place" => "複勝",
        "quinella" => "馬連",
        "wide" => "ワイド",
        "exacta" => "馬単",
        "trio" => "3連複",
        "trifecta" => "3連単",
        _ => slug,
    }
    .to_string()
}
