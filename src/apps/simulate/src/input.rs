//! 買い目定義 JSON のパースと、ドメインの `SimInput` への変換。

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use paddock_domain::{
    BetCombination, BetType, Finish, HorseNum, OrderedPair, OrderedTriple, Pair, PlacedBet,
    SimInput, Triple,
};
use serde::Deserialize;

/// シミュレータ入力 JSON のスキーマ。
#[derive(Debug, Deserialize)]
pub struct InputJson {
    /// 出走頭数。`field` 省略時に `1..=runners` を母集合にする。
    #[serde(default)]
    pub runners: Option<u32>,
    /// 出走馬の馬番を明示する場合（`runners` より優先）。
    #[serde(default)]
    pub field: Option<Vec<u32>>,
    /// 買い目集合。
    pub bets: Vec<BetJson>,
    /// 本線の着順 `[1着, 2着, 3着]`（任意。CLI `--main` で上書き可）。
    #[serde(default)]
    pub main: Option<Vec<u32>>,
    /// 各馬の単勝確率（馬番文字列→確率）。指定時のみ EV を算出する。
    #[serde(default)]
    pub win_probs: Option<HashMap<String, f64>>,
}

/// 1 つの買い目。
#[derive(Debug, Deserialize)]
pub struct BetJson {
    /// 券種（`win`/`place`/`quinella`/`wide`/`exacta`/`trio`/`trifecta` または日本語）。
    #[serde(rename = "type")]
    pub kind: String,
    /// 組合せの馬番（単勝/複勝=1, 馬連/ワイド/馬単=2, 三連複/三連単=3）。
    pub horses: Vec<u32>,
    /// 賭け金（円）。
    pub stake: u64,
    /// 払戻倍率。
    pub odds: f64,
}

fn horse(n: u32) -> Result<HorseNum> {
    HorseNum::try_from(n).map_err(|e| anyhow!(e))
}

fn build_combination(kind: BetType, horses: &[u32]) -> Result<BetCombination> {
    let need = |n: usize| -> Result<()> {
        if horses.len() != n {
            bail!(
                "{} には馬番 {} 個が必要ですが {} 個でした",
                kind.as_ja(),
                n,
                horses.len()
            );
        }
        Ok(())
    };
    Ok(match kind {
        BetType::Win => {
            need(1)?;
            BetCombination::Win(horse(horses[0])?)
        }
        BetType::Place => {
            need(1)?;
            BetCombination::Place(horse(horses[0])?)
        }
        BetType::Quinella => {
            need(2)?;
            BetCombination::Quinella(Pair::try_from((horse(horses[0])?, horse(horses[1])?))?)
        }
        BetType::Wide => {
            need(2)?;
            BetCombination::Wide(Pair::try_from((horse(horses[0])?, horse(horses[1])?))?)
        }
        BetType::Exacta => {
            need(2)?;
            BetCombination::Exacta(OrderedPair::try_from((horse(horses[0])?, horse(horses[1])?))?)
        }
        BetType::Trio => {
            need(3)?;
            BetCombination::Trio(Triple::try_from((
                horse(horses[0])?,
                horse(horses[1])?,
                horse(horses[2])?,
            ))?)
        }
        BetType::Trifecta => {
            need(3)?;
            BetCombination::Trifecta(OrderedTriple::try_from((
                horse(horses[0])?,
                horse(horses[1])?,
                horse(horses[2])?,
            ))?)
        }
    })
}

/// `"5-1-3"` 形式の着順文字列を `Finish` に変換する。
pub fn parse_finish(s: &str) -> Result<Finish> {
    let nums: Vec<u32> = s
        .split('-')
        .map(|t| t.trim().parse::<u32>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| anyhow!("着順 '{s}' を解釈できません: {e}"))?;
    if nums.len() != 3 {
        bail!("着順は '1着-2着-3着' の 3 つで指定してください（例: 5-1-3）");
    }
    Ok((horse(nums[0])?, horse(nums[1])?, horse(nums[2])?))
}

fn finish_from_vec(v: &[u32]) -> Result<Finish> {
    if v.len() != 3 {
        bail!("main は [1着, 2着, 3着] の 3 要素で指定してください");
    }
    Ok((horse(v[0])?, horse(v[1])?, horse(v[2])?))
}

/// JSON と CLI の本線指定から `SimInput` を構築する。`main_override` は CLI `--main`。
pub fn to_sim_input(json: InputJson, main_override: Option<&str>) -> Result<SimInput> {
    let field: Vec<HorseNum> = match &json.field {
        Some(nums) => nums.iter().map(|&n| horse(n)).collect::<Result<_>>()?,
        None => {
            let runners = json
                .runners
                .ok_or_else(|| anyhow!("`runners` か `field` のどちらかを指定してください"))?;
            (1..=runners).map(horse).collect::<Result<_>>()?
        }
    };

    let bets: Vec<PlacedBet> = json
        .bets
        .iter()
        .map(|b| {
            let kind = BetType::try_from(b.kind.as_str())
                .map_err(|e| anyhow!("未知の券種 '{}': {e}", b.kind))?;
            if !b.odds.is_finite() || b.odds < 1.0 {
                bail!(
                    "オッズは 1.0 以上の有限値で指定してください（{} odds={}）",
                    b.kind,
                    b.odds
                );
            }
            Ok(PlacedBet {
                combination: build_combination(kind, &b.horses)?,
                stake: b.stake,
                odds: b.odds,
            })
        })
        .collect::<Result<_>>()?;

    let main = match main_override {
        Some(s) => Some(parse_finish(s)?),
        None => match &json.main {
            Some(v) => Some(finish_from_vec(v)?),
            None => None,
        },
    };

    let win_probs = match json.win_probs {
        Some(map) => {
            let mut probs = HashMap::with_capacity(map.len());
            for (num, prob) in map {
                let n: u32 = num
                    .parse()
                    .map_err(|e| anyhow!("win_probs のキー '{num}' は馬番ではありません: {e}"))?;
                let hn = horse(n)?;
                if !field.contains(&hn) {
                    bail!("win_probs のキー {n} は field（出走馬）に存在しません");
                }
                if !prob.is_finite() || !(0.0..=1.0).contains(&prob) {
                    bail!("win_probs[{n}] は 0.0〜1.0 の有限値で指定してください（{prob}）");
                }
                probs.insert(hn, prob);
            }
            Some(probs)
        }
        None => None,
    };

    Ok(SimInput {
        field,
        bets,
        main,
        win_probs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample_json() {
        let raw = r#"
        { "runners": 6,
          "bets": [
            {"type":"wide","horses":[1,5],"stake":500,"odds":3.0},
            {"type":"三連単","horses":[1,5,8],"stake":100,"odds":50.0}
          ],
          "main": [1,5,2],
          "win_probs": {"1":0.5,"5":0.3} }
        "#;
        let json: InputJson = serde_json::from_str(raw).unwrap();
        let sim = to_sim_input(json, None).unwrap();
        assert_eq!(sim.field.len(), 6);
        assert_eq!(sim.bets.len(), 2);
        assert!(sim.main.is_some());
        assert!(sim.win_probs.is_some());
    }

    #[test]
    fn cli_main_overrides_json() {
        let raw = r#"{ "runners": 8, "bets": [], "main": [1,2,3] }"#;
        let json: InputJson = serde_json::from_str(raw).unwrap();
        let sim = to_sim_input(json, Some("5-1-3")).unwrap();
        let (a, b, c) = sim.main.unwrap();
        assert_eq!((a.value(), b.value(), c.value()), (5, 1, 3));
    }

    #[test]
    fn wrong_horse_count_errors() {
        let raw = r#"{ "runners": 6, "bets": [{"type":"win","horses":[1,2],"stake":100,"odds":2.0}] }"#;
        let json: InputJson = serde_json::from_str(raw).unwrap();
        assert!(to_sim_input(json, None).is_err());
    }

    #[test]
    fn win_probs_are_converted() {
        let raw = r#"{ "runners": 6, "bets": [], "win_probs": {"1":0.5,"5":0.3} }"#;
        let json: InputJson = serde_json::from_str(raw).unwrap();
        let sim = to_sim_input(json, None).unwrap();
        let probs = sim.win_probs.unwrap();
        assert_eq!(probs.len(), 2);
        assert_eq!(probs.get(&HorseNum::try_from(1).unwrap()).copied(), Some(0.5));
        assert_eq!(probs.get(&HorseNum::try_from(5).unwrap()).copied(), Some(0.3));
    }

    #[test]
    fn invalid_odds_errors() {
        let raw = r#"{ "runners": 6, "bets": [{"type":"win","horses":[1],"stake":100,"odds":0.5}] }"#;
        let json: InputJson = serde_json::from_str(raw).unwrap();
        assert!(to_sim_input(json, None).is_err());
    }

    #[test]
    fn parse_finish_rejects_bad_input() {
        assert!(parse_finish("1-2").is_err()); // 要素不足
        assert!(parse_finish("1-2-x").is_err()); // 非数値
        assert!(parse_finish("1-2-3").is_ok());
    }

    #[test]
    fn win_prob_out_of_range_errors() {
        let raw = r#"{ "runners": 6, "bets": [], "win_probs": {"1":1.5} }"#;
        let json: InputJson = serde_json::from_str(raw).unwrap();
        assert!(to_sim_input(json, None).is_err());
    }

    #[test]
    fn win_prob_key_outside_field_errors() {
        let raw = r#"{ "runners": 6, "bets": [], "win_probs": {"9":0.3} }"#;
        let json: InputJson = serde_json::from_str(raw).unwrap();
        assert!(to_sim_input(json, None).is_err());
    }
}
