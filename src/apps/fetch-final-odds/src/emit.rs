//! 保存済みの生 JSON から解析用 TSV を作る（ネットワーク不使用）。
//!
//! パースは本番と同じ `netkeiba_scraper::parse`、値域・番兵の判定はドメインの `OddsValue::try_from`
//! （券種必須・#630）を使う。二重実装を作らない（ADR 0064）。組合せキーは `race_odds` と同じ `to_key` 表記。
//!
//! 出力:
//! - `final_odds.tsv`: `race_id bet_type combination_key odds odds_high popularity`（ワイドだけ odds=下限・odds_high=上限）
//! - `index.tsv`: `race_id bet_type status official_datetime entries rows sentinel invalid unparsed`
//!   （entries = 応答の組合せ数、rows = TSV に出した数、unparsed = パーサが読み飛ばした数〈`---.-` 等〉）

use std::fmt::Write as _;
use std::path::Path;

use netkeiba_scraper::parse::{
    parse_odds_meta, parse_quinella_odds, parse_trio_odds, parse_wide_odds,
};
use paddock_domain::{BetType, Error as DomainError, OddsValue};

use crate::ODDS_TYPES;

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub bet_type: BetType,
    pub key: String,
    pub odds: f64,
    pub odds_high: Option<f64>,
    pub popularity: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileStats {
    pub bet_type: String,
    pub status: String,
    pub official_datetime: Option<String>,
    pub entries: usize,
    pub rows: usize,
    pub sentinel: usize,
    pub invalid: usize,
    pub unparsed: usize,
}

enum Verdict {
    Ok,
    Sentinel,
    Invalid,
}

fn check(bet_type: BetType, v: f64) -> Verdict {
    match OddsValue::try_from((bet_type, v)) {
        Ok(_) => Verdict::Ok,
        Err(DomainError::UnpricedSentinel(_)) => Verdict::Sentinel,
        Err(_) => Verdict::Invalid,
    }
}

/// 応答 1 本（race_id・type）を行に変換する。
pub fn emit_one(odds_type: u8, json: &str) -> anyhow::Result<(Vec<Row>, FileStats)> {
    let meta = parse_odds_meta(json)?;
    // index.tsv にそのまま書くので、列や行を壊す制御文字を含む値は受け付けない
    if let Some(t) = &meta.official_datetime {
        anyhow::ensure!(
            !t.chars().any(char::is_control),
            "official_datetime に制御文字が含まれています: {t:?}"
        );
    }
    let bet_type = ODDS_TYPES
        .iter()
        .find(|(t, _)| *t == odds_type)
        .map(|(_, b)| *b)
        .ok_or_else(|| anyhow::anyhow!("対象外の type です: {odds_type}"))?;
    let mut stats = FileStats {
        bet_type: bet_type.to_string(),
        status: meta.status,
        official_datetime: meta.official_datetime,
        entries: count_entries(json, odds_type)?,
        ..FileStats::default()
    };
    // 確定後（result）でない応答は確定オッズとして出さない（fetch は保存しないが、手で置かれた場合の防御）
    if stats.status != "result" {
        return Ok((Vec::new(), stats));
    }
    // (key, 下限 or 値, 上限, 人気)
    let parsed: Vec<(String, f64, Option<f64>, Option<u32>)> = match bet_type {
        BetType::Quinella => parse_quinella_odds(json)?
            .into_iter()
            .map(|o| (o.combination.to_key(), o.odds, None, o.popularity))
            .collect(),
        BetType::Wide => parse_wide_odds(json)?
            .into_iter()
            .map(|o| {
                (
                    o.combination.to_key(),
                    o.odds_low,
                    Some(o.odds_high),
                    o.popularity,
                )
            })
            .collect(),
        BetType::Trio => parse_trio_odds(json)?
            .into_iter()
            .map(|o| (o.combination.to_key(), o.odds, None, o.popularity))
            .collect(),
        _ => unreachable!("ODDS_TYPES は馬連・ワイド・三連複だけ"),
    };
    stats.unparsed = stats.entries.checked_sub(parsed.len()).ok_or_else(|| {
        anyhow::anyhow!(
            "パース結果（{}）が応答の組合せ数（{}）より多い",
            parsed.len(),
            stats.entries
        )
    })?;
    let mut rows = Vec::new();
    for (key, odds, odds_high, popularity) in parsed {
        let verdicts = [Some(odds), odds_high]
            .into_iter()
            .flatten()
            .map(|v| check(bet_type, v));
        let (mut sentinel, mut invalid) = (false, false);
        for v in verdicts {
            match v {
                Verdict::Ok => {}
                Verdict::Sentinel => sentinel = true,
                Verdict::Invalid => invalid = true,
            }
        }
        if odds_high.is_some_and(|h| h < odds) {
            invalid = true; // ワイドの下限 > 上限
        }
        if sentinel {
            stats.sentinel += 1;
        } else if invalid {
            stats.invalid += 1;
        } else {
            rows.push(Row {
                bet_type,
                key,
                odds,
                odds_high,
                popularity,
            });
        }
    }
    stats.rows = rows.len();
    Ok((rows, stats))
}

/// 応答の `data.odds["<type>"]` に載っている組合せの数（パーサが読み飛ばした行も含む）。
fn count_entries(json: &str, odds_type: u8) -> anyhow::Result<usize> {
    let root: serde_json::Value = serde_json::from_str(json)?;
    Ok(root
        .get("data")
        .and_then(|d| d.get("odds"))
        .and_then(|o| o.get(odds_type.to_string()))
        .and_then(|m| m.as_object())
        .map_or(0, |m| m.len()))
}

/// `<race_id>-<type>.json` を (race_id, type) に分ける。race_id 自体が `-` を含むので末尾で割る。
pub fn split_file_name(name: &str) -> Option<(String, u8)> {
    let stem = name.strip_suffix(".json")?;
    let (race_id, t) = stem.rsplit_once('-')?;
    // 手で置かれたファイルでも TSV に不正な race_id を書かないよう、ドメインの RaceId で検証する
    paddock_domain::RaceId::try_from(race_id).ok()?;
    Some((race_id.to_string(), t.parse().ok()?))
}

/// emit の集計（ファイル数・行数・組合せが 0 件の確定応答の数）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EmitSummary {
    pub files: usize,
    pub rows: usize,
    /// status=result なのに組合せが 0 件（中止など）。取得済みに数えられるが TSV に行が無い。
    pub empty_results: usize,
}

/// `<out>/raw` の全ファイルを読み、`final_odds.tsv` と `index.tsv` を書く。
/// 出力先はリポジトリの外に限る（[`crate::ensure_outside_repository`]）。
pub fn emit_dir(out_dir: &Path) -> anyhow::Result<EmitSummary> {
    crate::ensure_outside_repository(out_dir)?;
    let mut names: Vec<String> = std::fs::read_dir(out_dir.join("raw"))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    let mut odds_tsv =
        String::from("race_id\tbet_type\tcombination_key\todds\todds_high\tpopularity\n");
    let mut index_tsv = String::from(
        "race_id\tbet_type\tstatus\tofficial_datetime\tentries\trows\tsentinel\tinvalid\tunparsed\n",
    );
    let mut summary = EmitSummary::default();
    for name in names {
        let (race_id, odds_type) = split_file_name(&name)
            .ok_or_else(|| anyhow::anyhow!("想定外のファイル名です: {name}"))?;
        let json = std::fs::read_to_string(out_dir.join("raw").join(&name))?;
        let (rows, st) = emit_one(odds_type, &json).map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
        for r in &rows {
            writeln!(
                odds_tsv,
                "{race_id}\t{}\t{}\t{}\t{}\t{}",
                r.bet_type,
                r.key,
                r.odds,
                r.odds_high.map(|h| h.to_string()).unwrap_or_default(),
                r.popularity.map(|p| p.to_string()).unwrap_or_default(),
            )?;
        }
        writeln!(
            index_tsv,
            "{race_id}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            st.bet_type,
            st.status,
            st.official_datetime.unwrap_or_default(),
            st.entries,
            st.rows,
            st.sentinel,
            st.invalid,
            st.unparsed,
        )?;
        summary.files += 1;
        summary.rows += rows.len();
        if st.status == "result" && st.entries == 0 {
            summary.empty_results += 1;
        }
    }
    std::fs::write(out_dir.join("final_odds.tsv"), odds_tsv)?;
    std::fs::write(out_dir.join("index.tsv"), index_tsv)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUINELLA: &str = r#"{"status":"result","data":{"official_datetime":"2025-01-05 15:02:03","odds":{"4":{
        "0102":["1,141.1","0.0","3"],"0103":["5.6","0.0","1"],"0203":["99999.9","0.0","--"],"0104":["---.-","0.0",""]}}}}"#;
    // 確定後に取消・除外の馬を含む組合せは負の値（-3.0 / -2.0・人気 9999）で返る（実データで最多の除外経路）
    const QUINELLA_SCRATCHED: &str = r#"{"status":"result","data":{"odds":{"4":{
        "0102":["12.3","0.0","5"],"0111":["-3.0","0.0","9999"],"0211":["-2.0","0.0","9999"]}}}}"#;
    const WIDE: &str = r#"{"status":"result","data":{"odds":{"5":{
        "0102":["2.6","2.9","1"],"0103":["9999.9","0.0","--"],"0203":["4.0","3.0","2"]}}}}"#;
    const TRIO: &str = r#"{"status":"result","data":{"odds":{"7":{
        "010203":["29.9","0.0","5"],"010204":["9999.9","0.0","7"]}}}}"#;

    #[test]
    fn quinella_rows_drop_sentinel_and_count_unparsed() {
        let (rows, st) = emit_one(4, QUINELLA).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| (r.key.as_str(), r.odds, r.popularity))
                .collect::<Vec<_>>(),
            vec![("1-2", 1141.1, Some(3)), ("1-3", 5.6, Some(1))]
        );
        assert_eq!(st.status, "result");
        assert_eq!(st.official_datetime.as_deref(), Some("2025-01-05 15:02:03"));
        assert_eq!(
            (st.entries, st.rows, st.sentinel, st.invalid, st.unparsed),
            (4, 2, 1, 0, 1)
        );
    }

    #[test]
    fn negative_values_for_scratched_horses_are_counted_as_invalid() {
        let (rows, st) = emit_one(4, QUINELLA_SCRATCHED).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            vec!["1-2"]
        );
        assert_eq!((st.rows, st.sentinel, st.invalid), (1, 0, 2));
    }

    #[test]
    fn non_result_responses_emit_no_rows() {
        let middle = QUINELLA.replacen(r#""status":"result""#, r#""status":"middle""#, 1);
        let (rows, st) = emit_one(4, &middle).unwrap();
        assert!(rows.is_empty());
        assert_eq!(
            (st.status.as_str(), st.bet_type.as_str(), st.entries),
            ("middle", "quinella", 4)
        );
    }

    #[test]
    fn wide_keeps_band_and_rejects_sentinel_or_inverted_band() {
        let (rows, st) = emit_one(5, WIDE).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].key.as_str(), rows[0].odds, rows[0].odds_high),
            ("1-2", 2.6, Some(2.9))
        );
        // 9999.9 はワイドの番兵（相方 0.0 は値域違反だが番兵を優先して数える）。4.0 > 3.0 は下限 > 上限
        assert_eq!((st.sentinel, st.invalid), (1, 1));
    }

    #[test]
    fn trio_keeps_9999_9_because_sentinels_are_per_bet_type() {
        // 三連複の番兵は 99999.9。9999.9 は正当な配当なので落とさない（#630）
        let (rows, st) = emit_one(7, TRIO).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| (r.key.as_str(), r.odds))
                .collect::<Vec<_>>(),
            vec![("1-2-3", 29.9), ("1-2-4", 9999.9)]
        );
        assert_eq!(st.sentinel, 0);
    }

    #[test]
    fn control_characters_in_official_datetime_are_rejected() {
        let bad = r#"{"status":"result","data":{"official_datetime":"2025-01-05\t15:02","odds":{"4":{}}}}"#;
        assert!(emit_one(4, bad).is_err());
    }

    #[test]
    fn file_names_split_at_the_last_dash() {
        assert_eq!(
            split_file_name("2025-1-chukyo-1-10R-7.json"),
            Some(("2025-1-chukyo-1-10R".into(), 7))
        );
        assert_eq!(split_file_name("2025-1-chukyo-1-10R-7.json.tmp"), None);
        assert_eq!(split_file_name("../evil-4.json"), None); // RaceId として不正
    }

    #[test]
    fn emit_dir_writes_both_tsvs_and_ignores_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw");
        std::fs::create_dir_all(&raw).unwrap();
        std::fs::write(raw.join("2025-1-chukyo-1-10R-4.json"), QUINELLA).unwrap();
        std::fs::write(raw.join("2025-1-chukyo-1-10R-7.json"), TRIO).unwrap();
        std::fs::write(raw.join("2025-1-chukyo-1-10R-5.json.tmp"), "partial").unwrap();
        std::fs::write(
            raw.join("2025-1-chukyo-1-11R-4.json"),
            r#"{"status":"result","data":{"odds":{}}}"#,
        )
        .unwrap();
        assert_eq!(
            emit_dir(dir.path()).unwrap(),
            EmitSummary {
                files: 3,
                rows: 4,
                empty_results: 1
            }
        );
        let odds = std::fs::read_to_string(dir.path().join("final_odds.tsv")).unwrap();
        assert_eq!(
            odds,
            "race_id\tbet_type\tcombination_key\todds\todds_high\tpopularity\n\
             2025-1-chukyo-1-10R\tquinella\t1-2\t1141.1\t\t3\n\
             2025-1-chukyo-1-10R\tquinella\t1-3\t5.6\t\t1\n\
             2025-1-chukyo-1-10R\ttrio\t1-2-3\t29.9\t\t5\n\
             2025-1-chukyo-1-10R\ttrio\t1-2-4\t9999.9\t\t7\n"
        );
        let index = std::fs::read_to_string(dir.path().join("index.tsv")).unwrap();
        assert!(
            index.contains(
                "2025-1-chukyo-1-10R\tquinella\tresult\t2025-01-05 15:02:03\t4\t2\t1\t0\t1\n"
            ),
            "{index}"
        );
    }
}
