use paddock_domain::race::{Race, Surface, Venue};

#[derive(Debug, Clone, PartialEq)]
pub struct RaceRow {
    pub race_id: String,
    pub race_num: u32,
    pub venue: String,
    pub surface: String,
    pub distance: u32,
}

impl From<&Race> for RaceRow {
    fn from(r: &Race) -> Self {
        Self {
            race_id: r.race_id.value().to_string(),
            race_num: r.race_num,
            venue: venue_jp(&r.venue),
            surface: surface_jp(&r.surface).to_string(),
            distance: r.distance,
        }
    }
}

fn surface_jp(s: &Surface) -> &'static str {
    match s.as_str() {
        "turf" => "芝",
        "dirt" => "ダ",
        other => other,
    }
}

fn venue_jp(v: &Venue) -> String {
    match v.as_slug() {
        "sapporo" => "札幌",
        "hakodate" => "函館",
        "fukushima" => "福島",
        "niigata" => "新潟",
        "tokyo" => "東京",
        "nakayama" => "中山",
        "chukyo" => "中京",
        "kyoto" => "京都",
        "hanshin" => "阪神",
        "kokura" => "小倉",
        _ => v.as_slug(),
    }
    .to_string()
}
