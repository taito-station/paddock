//! 予想（印・短評・買い目・結果）の編集物エンティティ。
//!
//! これまで予想は Obsidian の MD にしか無かったが、本エンティティを構造化レコードとして
//! DB に永続化し（DB が正）、MD はここから生成する“ビュー”に位置づける。確率推定の
//! [`crate::prediction`]（モデル出力）とは別物で、こちらは印・短評・最終買い目・結果という
//! 編集物を保持する。

use chrono::NaiveDate;

use crate::race::Venue;

/// 予想印。競馬の本命〜注の 6 種。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// ◎ 本命
    Honmei,
    /// ○ 対抗
    Taikou,
    /// ▲ 単穴
    Tanana,
    /// △ 連下
    Renge,
    /// ☆ 穴
    Hoshi,
    /// 注 注意
    Chui,
}

impl Mark {
    /// 表示記号（◎○▲△☆注）。
    pub fn as_symbol(&self) -> &'static str {
        match self {
            Mark::Honmei => "◎",
            Mark::Taikou => "○",
            Mark::Tanana => "▲",
            Mark::Renge => "△",
            Mark::Hoshi => "☆",
            Mark::Chui => "注",
        }
    }

    /// DB/JSON 用の slug（honmei/taikou/tanana/renge/hoshi/chui）。
    pub fn as_slug(&self) -> &'static str {
        match self {
            Mark::Honmei => "honmei",
            Mark::Taikou => "taikou",
            Mark::Tanana => "tanana",
            Mark::Renge => "renge",
            Mark::Hoshi => "hoshi",
            Mark::Chui => "chui",
        }
    }

    /// 記号（◎ 等）から印を引く。
    pub fn from_symbol(s: &str) -> Option<Mark> {
        Some(match s.trim() {
            "◎" => Mark::Honmei,
            "○" | "〇" => Mark::Taikou,
            "▲" => Mark::Tanana,
            "△" => Mark::Renge,
            "☆" => Mark::Hoshi,
            "注" => Mark::Chui,
            _ => return None,
        })
    }

    /// slug（honmei 等）から印を引く。記号も受け付ける。
    pub fn from_slug(s: &str) -> Option<Mark> {
        let t = s.trim();
        Some(match t {
            "honmei" => Mark::Honmei,
            "taikou" => Mark::Taikou,
            "tanana" => Mark::Tanana,
            "renge" => Mark::Renge,
            "hoshi" => Mark::Hoshi,
            "chui" => Mark::Chui,
            _ => return Mark::from_symbol(t),
        })
    }
}

/// 予想 1 頭分（印・確率・単勝/人気・短評）。確率/単勝/人気は不明なら `None`。
#[derive(Debug, Clone)]
pub struct PredictionHorse {
    pub horse_num: u32,
    pub horse_name: String,
    pub jockey: Option<String>,
    pub mark: Option<Mark>,
    pub win_odds: Option<f64>,
    pub popularity: Option<u32>,
    pub win_prob: Option<f64>,
    pub place_prob: Option<f64>,
    pub show_prob: Option<f64>,
    pub comment: Option<String>,
}

/// 買い目 1 点。`combination` は arabic 馬番のハイフン連結（"7" / "7-14" / "7-14-13"）。
#[derive(Debug, Clone)]
pub struct PredictionBet {
    pub bet_type: String,
    pub combination: String,
    pub amount: u64,
}

/// レース結果（答え合わせ後にのみ付く）。`finish` は 1〜3 着の馬番。
#[derive(Debug, Clone, Default)]
pub struct PredictionResult {
    pub finish: [Option<u32>; 3],
    pub recovery_rate: Option<f64>,
    pub pnl: Option<i64>,
    pub note: Option<String>,
}

/// 1 レース分の予想。`(date, venue, race_num)` で一意。
#[derive(Debug, Clone)]
pub struct PadPrediction {
    pub date: NaiveDate,
    pub venue: Venue,
    pub race_num: u32,
    pub title: Option<String>,
    pub budget: Option<u64>,
    pub strategy_note: Option<String>,
    pub commentary: Option<String>,
    pub horses: Vec<PredictionHorse>,
    pub bets: Vec<PredictionBet>,
    pub result: Option<PredictionResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_symbol_slug_roundtrip() {
        for m in [
            Mark::Honmei,
            Mark::Taikou,
            Mark::Tanana,
            Mark::Renge,
            Mark::Hoshi,
            Mark::Chui,
        ] {
            assert_eq!(Mark::from_symbol(m.as_symbol()), Some(m));
            assert_eq!(Mark::from_slug(m.as_slug()), Some(m));
        }
    }

    #[test]
    fn from_slug_accepts_symbol_and_variant() {
        assert_eq!(Mark::from_slug("◎"), Some(Mark::Honmei));
        assert_eq!(Mark::from_symbol("〇"), Some(Mark::Taikou)); // 全角まる異体
        assert_eq!(Mark::from_slug("unknown"), None);
    }
}
