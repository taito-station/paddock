//! DB の予想（`PadPrediction`）から pad の MD を生成する。DB が正で MD は生成物。
//! レイアウトは web-viewer（#143）が綺麗に出せる形（印テーブル＋買い目テーブル＋結果 callout）。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use paddock_domain::PadPrediction;

/// `pad_root/{YYYYMMDD}/{venue_slug}/{NN}R.md` に MD を書き出し、書いたパスを返す。
pub fn write_md(pad_root: &Path, p: &PadPrediction) -> Result<PathBuf> {
    let dir = pad_root
        .join(p.date.format("%Y%m%d").to_string())
        .join(p.venue.as_slug());
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("ディレクトリ作成に失敗: {}", dir.display()))?;
    let path = dir.join(format!("{:02}R.md", p.race_num));
    std::fs::write(&path, render_md(p))
        .with_context(|| format!("MD 書き込みに失敗: {}", path.display()))?;
    Ok(path)
}

/// 予想を Markdown 文字列にする。
pub fn render_md(p: &PadPrediction) -> String {
    let mut s = String::new();

    // 見出し
    s.push_str(&format!(
        "# {} {}{}R",
        p.date.format("%Y-%m-%d"),
        p.venue.as_jp(),
        p.race_num
    ));
    if let Some(t) = &p.title {
        s.push_str(&format!(" {}", t.trim()));
    }
    s.push_str("\n\n");

    if let Some(b) = p.budget {
        s.push_str(&format!("- 予算: {}円\n\n", group_thousands(b)));
    }

    // 印と短評。読みやすさのため印の優先度（◎○▲△☆注→無印）順、同順位は馬番昇順で並べる。
    let mut horses: Vec<&_> = p.horses.iter().collect();
    horses.sort_by_key(|h| (mark_rank(h.mark), h.horse_num));

    s.push_str("## 印と短評\n\n");
    s.push_str("| 印 | 馬番 | 馬名 | 騎手 | 単勝 | 人気 | 勝率 | 連対率 | 複勝率 | 短評 |\n");
    s.push_str("|---|---|---|---|---|---|---|---|---|---|\n");
    for h in horses {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            h.mark.map(|m| m.as_symbol()).unwrap_or(""),
            h.horse_num,
            cell(&h.horse_name),
            opt(h.jockey.as_deref()),
            odds(h.win_odds),
            h.popularity
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".into()),
            pct(h.win_prob),
            pct(h.place_prob),
            pct(h.show_prob),
            opt(h.comment.as_deref()),
        ));
    }
    s.push('\n');

    // 買い目
    s.push_str("## 買い目\n\n");
    s.push_str("| 券種 | 買い目 | 金額 |\n|---|---|---|\n");
    let mut total = 0u64;
    for b in &p.bets {
        total += b.amount;
        s.push_str(&format!(
            "| {} | {} | {} |\n",
            cell(&b.bet_type),
            cell(&b.combination),
            group_thousands(b.amount)
        ));
    }
    if !p.bets.is_empty() {
        s.push_str(&format!(
            "| **合計** |  | **{}** |\n",
            group_thousands(total)
        ));
    }
    s.push('\n');

    if let Some(note) = &p.strategy_note {
        s.push_str(note.trim());
        s.push_str("\n\n");
    }

    // 結果 callout
    if let Some(r) = &p.result {
        let mut parts = Vec::new();
        let finish: Vec<String> = r
            .finish
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.map(|n| format!("{}着 {}", i + 1, n)))
            .collect();
        if !finish.is_empty() {
            parts.push(finish.join(" / "));
        }
        if let Some(rr) = r.recovery_rate {
            parts.push(format!("回収率 {rr}%"));
        }
        if let Some(pnl) = r.pnl {
            parts.push(format!("収支 {pnl}"));
        }
        s.push_str(&format!("> 結果: {}\n", parts.join(" | ")));
        if let Some(note) = &r.note {
            s.push_str(&format!(">\n> {}\n", note.trim().replace('\n', "\n> ")));
        }
        s.push('\n');
    }

    if let Some(c) = &p.commentary {
        s.push_str(c.trim());
        s.push('\n');
    }

    s
}

/// 印の表示優先度（◎=0 … 注=5、無印=6）。
fn mark_rank(mark: Option<paddock_domain::Mark>) -> u8 {
    use paddock_domain::Mark::*;
    match mark {
        Some(Honmei) => 0,
        Some(Taikou) => 1,
        Some(Tanana) => 2,
        Some(Renge) => 3,
        Some(Hoshi) => 4,
        Some(Chui) => 5,
        None => 6,
    }
}

/// セル内の `|` と改行はテーブルを壊すため置換する。
fn cell(s: &str) -> String {
    s.replace('|', "/").replace('\n', " ")
}

fn opt(s: Option<&str>) -> String {
    match s {
        Some(v) => cell(v),
        None => "-".into(),
    }
}

fn pct(v: Option<f64>) -> String {
    match v {
        Some(v) => format!("{v}%"),
        None => "-".into(),
    }
}

fn odds(v: Option<f64>) -> String {
    match v {
        Some(v) => format!("{v}"),
        None => "-".into(),
    }
}

/// 1000 区切り（例: 10000 -> "10,000"）。
fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use paddock_domain::{Mark, PredictionBet, PredictionHorse, PredictionResult, Venue};

    fn sample() -> PadPrediction {
        PadPrediction {
            date: NaiveDate::from_ymd_opt(2026, 6, 13).unwrap(),
            venue: Venue::Hanshin,
            race_num: 4,
            title: Some("3歳未勝利".into()),
            budget: Some(10000),
            strategy_note: Some("人気軸＋相手広め".into()),
            commentary: None,
            horses: vec![PredictionHorse {
                horse_num: 7,
                horse_name: "ラパンドール".into(),
                jockey: Some("松山".into()),
                mark: Some(Mark::Honmei),
                win_odds: Some(2.4),
                popularity: Some(1),
                win_prob: Some(25.4),
                place_prob: Some(25.4),
                show_prob: Some(25.4),
                comment: Some("単独最上位".into()),
            }],
            bets: vec![
                PredictionBet {
                    bet_type: "単勝".into(),
                    combination: "7".into(),
                    amount: 600,
                },
                PredictionBet {
                    bet_type: "馬連".into(),
                    combination: "7-14".into(),
                    amount: 1000,
                },
            ],
            result: Some(PredictionResult {
                finish: [Some(7), Some(4), Some(13)],
                recovery_rate: Some(52.1),
                pnl: Some(-4790),
                note: Some("印は上位3頭捕捉".into()),
            }),
        }
    }

    #[test]
    fn renders_expected_sections() {
        let md = render_md(&sample());
        assert!(md.contains("# 2026-06-13 阪神4R 3歳未勝利"));
        assert!(md.contains("## 印と短評"));
        assert!(md.contains(
            "| ◎ | 7 | ラパンドール | 松山 | 2.4 | 1 | 25.4% | 25.4% | 25.4% | 単独最上位 |"
        ));
        assert!(md.contains("## 買い目"));
        assert!(md.contains("| **合計** |  | **1,600** |"));
        assert!(md.contains("> 結果: 1着 7 / 2着 4 / 3着 13 | 回収率 52.1% | 収支 -4790"));
    }

    #[test]
    fn thousands_grouping() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(600), "600");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(10000), "10,000");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }
}
