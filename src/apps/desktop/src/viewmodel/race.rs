use chrono::{NaiveDate, NaiveTime};
use paddock_use_case::interactor::race::board::{BoardHorse, RaceBoard};

use crate::viewmodel::exec::{BetView, portfolio_to_bets};

#[derive(Debug, Clone, PartialEq)]
pub struct BoardView {
    pub race_id: String,
    pub date: NaiveDate,
    pub venue: String,
    pub race_num: u32,
    pub surface: String,
    pub distance: u32,
    pub post_time: Option<String>,
    pub race_name: Option<String>,
    pub race_class: Option<String>,
    pub is_confused: bool,
    pub axis_win_prob: f64,
    pub qualifying_count: u32,
    pub result_confirmed: bool,
    pub bets: Vec<BetView>,
    pub post_time_raw: Option<NaiveTime>,
    pub horses: Vec<HorseView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HorseView {
    pub horse_num: u32,
    pub horse_name: String,
    pub jockey: String,
    pub win_prob: String,
    pub pure_win_prob: String,
    pub market_implied: String,
    pub win_odds: String,
    pub model_rank: u32,
    pub mark: String,
    pub is_overlay: bool,
    pub is_value: bool,
    pub finishing_position: Option<u32>,
    pub comment: Option<String>,
}

impl From<&RaceBoard> for BoardView {
    fn from(b: &RaceBoard) -> Self {
        Self {
            race_id: b.race_id.value().to_string(),
            date: b.date,
            venue: b.venue.clone(),
            race_num: b.race_num,
            surface: b.surface.clone(),
            distance: b.distance,
            post_time: b.post_time.clone(),
            race_name: b.race_name.clone(),
            race_class: b.race_class.clone(),
            is_confused: b.confusion.is_confused,
            axis_win_prob: b.confusion.axis_win_prob,
            qualifying_count: b.confusion.qualifying_count,
            result_confirmed: b.result_confirmed,
            bets: b
                .portfolio
                .as_ref()
                .map(portfolio_to_bets)
                .unwrap_or_default(),
            post_time_raw: b
                .post_time
                .as_deref()
                .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok()),
            horses: b.horses.iter().map(HorseView::from).collect(),
        }
    }
}

impl From<&BoardHorse> for HorseView {
    fn from(h: &BoardHorse) -> Self {
        Self {
            horse_num: h.horse_num,
            horse_name: h.horse_name.clone(),
            jockey: h.jockey.clone().unwrap_or_default(),
            win_prob: format!("{:.1}%", h.win_prob * 100.0),
            pure_win_prob: format!("{:.1}%", h.pure_win_prob * 100.0),
            market_implied: h
                .market_implied
                .map(|p| format!("{:.1}%", p * 100.0))
                .unwrap_or_else(|| "—".to_string()),
            win_odds: h
                .win_odds
                .map(|o| format!("{:.1}", o))
                .unwrap_or_else(|| "—".to_string()),
            model_rank: h.model_rank,
            mark: mark_symbol(h.mark.as_deref()),
            is_overlay: h.is_overlay,
            is_value: h.is_value,
            finishing_position: h.finishing_position,
            comment: h.comment.clone(),
        }
    }
}

fn mark_symbol(slug: Option<&str>) -> String {
    match slug {
        Some("honmei") => "◎",
        Some("taikou") => "○",
        Some("tanana") => "▲",
        Some("hoshi") => "☆",
        Some("renka") => "△",
        Some("chui") => "注",
        _ => "",
    }
    .to_string()
}
