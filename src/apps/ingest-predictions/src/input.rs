//! 予想 JSON のスキーマと、ドメイン `PadPrediction` への変換。
//!
//! 予想を作る Claude はこの JSON を吐けばよい（MD 整形は不要）。確率・単勝・人気は
//! 不明なら省略（null）。`win_prob`/`place_prob`/`show_prob` は **百分率の表示値**
//! （例 `25.4` = 25.4%）で受け取り、そのまま保持・表示する。
//!
//! 入力 DTO（serde 構造体）と `to_domain` の検証は、JSON を stdin で受ける既存の
//! `simulate` バイナリと同じく app 層に置く（domain を serde 非依存に保つため）。

use anyhow::{Context, Result, anyhow, bail};
use chrono::NaiveDate;
use paddock_domain::{
    Mark, PadPrediction, PredictionBet, PredictionHorse, PredictionResult, Venue,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Input {
    Many(Vec<PredictionJson>),
    // 単一 variant は大きいので Box 化（enum のサイズ差を抑える）。
    One(Box<PredictionJson>),
}

#[derive(Debug, Deserialize)]
struct PredictionJson {
    date: String,
    venue: String,
    race_num: u32,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    budget: Option<u64>,
    #[serde(default)]
    strategy_note: Option<String>,
    #[serde(default)]
    commentary: Option<String>,
    horses: Vec<HorseJson>,
    #[serde(default)]
    bets: Vec<BetJson>,
    #[serde(default)]
    result: Option<ResultJson>,
}

#[derive(Debug, Deserialize)]
struct HorseJson {
    horse_num: u32,
    horse_name: String,
    #[serde(default)]
    jockey: Option<String>,
    /// 印。slug（honmei…）または記号（◎…）。
    #[serde(default)]
    mark: Option<String>,
    #[serde(default)]
    win_odds: Option<f64>,
    #[serde(default)]
    popularity: Option<u32>,
    #[serde(default)]
    win_prob: Option<f64>,
    #[serde(default)]
    place_prob: Option<f64>,
    #[serde(default)]
    show_prob: Option<f64>,
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BetJson {
    bet_type: String,
    combination: String,
    amount: u64,
}

#[derive(Debug, Deserialize)]
struct ResultJson {
    /// 1〜3 着の馬番（先頭から順に）。3 要素未満なら残りは未確定として扱う。
    #[serde(default)]
    finish: Vec<u32>,
    #[serde(default)]
    recovery_rate: Option<f64>,
    #[serde(default)]
    pnl: Option<i64>,
    #[serde(default)]
    note: Option<String>,
}

/// 1 件または配列の予想 JSON をドメイン型へ変換する。
pub fn parse(raw: &str) -> Result<Vec<PadPrediction>> {
    let input: Input = serde_json::from_str(raw).context("予想 JSON の解釈に失敗")?;
    let jsons = match input {
        Input::Many(v) => v,
        Input::One(p) => vec![*p],
    };
    jsons.into_iter().map(to_domain).collect()
}

fn to_domain(j: PredictionJson) -> Result<PadPrediction> {
    let date = NaiveDate::parse_from_str(&j.date, "%Y-%m-%d")
        .map_err(|e| anyhow!("date '{}' を解釈できません（YYYY-MM-DD）: {e}", j.date))?;
    let venue = Venue::try_from(j.venue.as_str())
        .map_err(|e| anyhow!("venue '{}' を解釈できません: {e}", j.venue))?;

    let horses = j
        .horses
        .into_iter()
        .map(|h| {
            let mark = match h.mark {
                Some(s) => Some(
                    Mark::from_slug(&s)
                        .ok_or_else(|| anyhow!("印 '{s}' を解釈できません（◎○▲△☆注 か slug）"))?,
                ),
                None => None,
            };
            Ok(PredictionHorse {
                horse_num: h.horse_num,
                horse_name: h.horse_name,
                jockey: h.jockey,
                mark,
                win_odds: h.win_odds,
                popularity: h.popularity,
                win_prob: h.win_prob,
                place_prob: h.place_prob,
                show_prob: h.show_prob,
                comment: h.comment,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let bets = j
        .bets
        .into_iter()
        .map(|b| PredictionBet {
            bet_type: b.bet_type,
            combination: b.combination,
            amount: b.amount,
        })
        .collect();

    let result = match j.result {
        Some(r) => {
            if r.finish.len() > 3 {
                bail!("result.finish は最大 3 要素（1〜3 着）です");
            }
            let mut finish = [None, None, None];
            for (i, n) in r.finish.iter().enumerate() {
                finish[i] = Some(*n);
            }
            Some(PredictionResult {
                finish,
                recovery_rate: r.recovery_rate,
                pnl: r.pnl,
                note: r.note,
            })
        }
        None => None,
    };

    Ok(PadPrediction {
        date,
        venue,
        race_num: j.race_num,
        title: j.title,
        budget: j.budget,
        strategy_note: j.strategy_note,
        commentary: j.commentary,
        horses,
        bets,
        result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
    {
      "date": "2026-06-13",
      "venue": "hanshin",
      "race_num": 4,
      "title": "3歳未勝利",
      "budget": 10000,
      "strategy_note": "人気軸＋相手広め",
      "horses": [
        {"horse_num":7,"horse_name":"ラパンドール","jockey":"松山","mark":"◎",
         "win_odds":2.4,"popularity":1,"win_prob":25.4,"place_prob":25.4,"show_prob":25.4,
         "comment":"単独最上位"},
        {"horse_num":4,"horse_name":"ファランギーナ","jockey":"松若","mark":"renge",
         "win_odds":13.2,"popularity":6,"win_prob":6.1,"place_prob":15.2,"show_prob":21.4}
      ],
      "bets": [
        {"bet_type":"単勝","combination":"7","amount":600},
        {"bet_type":"馬連","combination":"7-14","amount":1000}
      ],
      "result": {"finish":[7,4,13],"recovery_rate":52.1,"pnl":-4790,"note":"印は上位3頭捕捉"}
    }
    "#;

    #[test]
    fn parses_single_object() {
        let preds = parse(SAMPLE).unwrap();
        assert_eq!(preds.len(), 1);
        let p = &preds[0];
        assert_eq!(p.venue, Venue::Hanshin);
        assert_eq!(p.race_num, 4);
        assert_eq!(p.horses.len(), 2);
        assert_eq!(p.horses[0].mark, Some(Mark::Honmei));
        assert_eq!(p.horses[1].mark, Some(Mark::Renge));
        assert_eq!(p.bets.len(), 2);
        let r = p.result.as_ref().unwrap();
        assert_eq!(r.finish, [Some(7), Some(4), Some(13)]);
        assert_eq!(r.pnl, Some(-4790));
    }

    #[test]
    fn parses_array() {
        let raw = format!("[{SAMPLE},{SAMPLE}]");
        assert_eq!(parse(&raw).unwrap().len(), 2);
    }

    #[test]
    fn rejects_unknown_mark() {
        let raw = r#"{"date":"2026-06-13","venue":"中山","race_num":1,
            "horses":[{"horse_num":1,"horse_name":"x","mark":"bogus"}],"bets":[]}"#;
        assert!(parse(raw).is_err());
    }

    #[test]
    fn rejects_bad_venue() {
        let raw = r#"{"date":"2026-06-13","venue":"nowhere","race_num":1,"horses":[],"bets":[]}"#;
        assert!(parse(raw).is_err());
    }
}
