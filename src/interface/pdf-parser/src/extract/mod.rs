mod header;
pub mod jockey_stext;
mod row;

use paddock_domain::{
    FinishingPosition, GateNum, HorseName, HorseNum, HorseResult, JockeyName, Race, RaceId,
    ResultStatus, TimeSeconds, TrainerName,
};

use crate::error::Result;

pub use header::RaceHeader;
pub use jockey_stext::{JockeyIndex, TrainerIndex, WeightIndex};
pub use row::RawRow;

/// 騎手・調教師・斤量は stext 座標ベースの索引（race_num→horse_num→値）で確定する。
/// 索引が空（stext 抽出失敗）または該当馬が無い場合は、各行の既存ヒューリスティックに
/// フォールバックする（現行挙動から後退させない）。人気は単勝オッズの昇順順位から算出する（#124）。
pub fn parse_text(
    text: &str,
    jockeys: &JockeyIndex,
    trainers: &TrainerIndex,
    weights: &WeightIndex,
) -> Result<Vec<Race>> {
    let blocks = split_into_race_blocks(text);
    let mut races = Vec::with_capacity(blocks.len());
    for block in blocks {
        if let Some(race) = build_race_from_block(&block, jockeys, trainers, weights)? {
            races.push(race);
        }
    }
    Ok(races)
}

/// A race block is the slice of lines from one race-start marker to the next.
fn split_into_race_blocks(text: &str) -> Vec<Vec<String>> {
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            if header::is_race_start_line(l) {
                Some(i)
            } else {
                None
            }
        })
        .collect();
    if starts.is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::with_capacity(starts.len());
    for (idx, &start) in starts.iter().enumerate() {
        let end = starts.get(idx + 1).copied().unwrap_or(lines.len());
        blocks.push(lines[start..end].to_vec());
    }
    blocks
}

fn build_race_from_block(
    lines: &[String],
    jockeys: &JockeyIndex,
    trainers: &TrainerIndex,
    weights: &WeightIndex,
) -> Result<Option<Race>> {
    let header = match header::parse_header(lines)? {
        Some(h) => h,
        None => return Ok(None),
    };

    let race_id_str = format!(
        "{}-{}-{}-{}-{}R",
        header.year,
        header.round,
        header.venue.as_slug(),
        header.day,
        header.race_num
    );
    let race_id = RaceId::try_from(race_id_str)?;

    let chunks = row::collect_chunks(lines);
    let field_size = header::find_field_size(lines);

    let mut results = Vec::with_capacity(chunks.len());
    let mut finisher_count: u32 = 0;
    let mut valid_chunk_idx: u32 = 0;
    let mut previous_time: Option<TimeSeconds> = None;
    for chunk in chunks.iter() {
        let raw = row::parse_chunk(chunk);
        let gate_num = match raw.gate.and_then(|n| GateNum::try_from(n).ok()) {
            Some(g) => g,
            None => continue,
        };
        let horse_num = match raw.horse_num.and_then(|n| HorseNum::try_from(n).ok()) {
            Some(h) => h,
            None => continue,
        };
        let horse_name = match raw
            .horse_name
            .as_deref()
            .and_then(|s| HorseName::try_from(s).ok())
        {
            Some(n) => n,
            None => continue,
        };

        let status = chunk_status(chunk);
        let beyond_finishers = field_size.is_some_and(|n| valid_chunk_idx >= n);
        valid_chunk_idx += 1;

        let finishing_position = if status != ResultStatus::Finished || beyond_finishers {
            None
        } else {
            finisher_count += 1;
            Some(FinishingPosition::try_from(finisher_count)?)
        };

        // 騎手は stext 座標ベースの索引（race_num, horse_num）を優先し、
        // 無い場合のみ既存のテキストヒューリスティックにフォールバックする。
        let jockey = jockeys
            .get(&header.race_num)
            .and_then(|m| m.get(&horse_num.value()))
            .and_then(|s| JockeyName::try_from(s.as_str()).ok())
            .or_else(|| {
                raw.jockey
                    .as_deref()
                    .and_then(|s| JockeyName::try_from(s).ok())
            });
        // 調教師も stext 座標索引を優先し、無い場合のみテキストヒューリスティックに
        // フォールバックする（現状 row 側の guess_trainer は None 固定）。
        let trainer = trainers
            .get(&header.race_num)
            .and_then(|m| m.get(&horse_num.value()))
            .and_then(|s| TrainerName::try_from(s.as_str()).ok())
            .or_else(|| {
                raw.trainer
                    .as_deref()
                    .and_then(|s| TrainerName::try_from(s).ok())
            });
        let time_seconds = raw
            .time_str
            .as_deref()
            .and_then(|s| TimeSeconds::try_from_mss_str(s).ok())
            .or(if raw.time_inherits {
                previous_time
            } else {
                None
            });
        if let Some(t) = time_seconds {
            previous_time = Some(t);
        }

        results.push(HorseResult {
            finishing_position,
            status,
            gate_num,
            horse_num,
            horse_name,
            horse_id: None,
            jockey,
            trainer,
            time_seconds,
            margin: raw.margin,
            odds: raw.odds,
            horse_weight: raw.horse_weight,
            weight_change: raw.weight_change,
            // 斤量は stext 座標索引（CID 数字）で確定する（#124）。索引に無い行は None。
            weight_carried: weights
                .get(&header.race_num)
                .and_then(|m| m.get(&horse_num.value()))
                .copied(),
            popularity: None,
        });
    }

    // 人気は単勝オッズの昇順順位から算出する（EdiF 列の復号に依らず決定的、#124）。
    assign_popularity_from_odds(&mut results);

    let race = Race {
        race_id,
        date: header.date,
        venue: header.venue,
        round: header.round,
        day: header.day,
        race_num: header.race_num,
        surface: header.surface,
        distance: header.distance,
        track_condition: header.track_condition,
        weather: header.weather,
        results,
    };
    Ok(Some(race))
}

/// 単勝オッズの昇順順位を人気として各結果に割り当てる（#124）。
///
/// JRA の人気は単勝オッズの低い順（＝支持の高い順）で決まる。順位付けの母数は status を問わず
/// 全 results だが、`popularity_ranks` は `odds` が `Some` の行だけを対象にする。すなわち
/// **確定オッズを持つ出走馬（競走中止 DNF も含む）が対象**で、確定オッズを持たない出走取消・
/// 競走除外（`odds == None`）は自然に母数から外れる（行は残るが人気は None）。
/// 同オッズは同順位とする（競争順位＝`1,2,2,4`）。
fn assign_popularity_from_odds(results: &mut [HorseResult]) {
    let ranks = popularity_ranks(&results.iter().map(|r| r.odds).collect::<Vec<_>>());
    for (result, rank) in results.iter_mut().zip(ranks) {
        result.popularity = rank;
    }
}

/// オッズ列 → 人気順位列（純関数）。オッズ昇順の競争順位（同値同順位 `1,2,2,4`）。`None` は据え置き。
fn popularity_ranks(odds: &[Option<f64>]) -> Vec<Option<u32>> {
    let mut ranked: Vec<(usize, f64)> = odds
        .iter()
        .enumerate()
        // NaN は順位を壊すため除外する（純関数としての防御。実データでは想定しない）。
        .filter_map(|(i, o)| o.filter(|v| v.is_finite()).map(|v| (i, v)))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut out = vec![None; odds.len()];
    let mut prev_odds: Option<f64> = None;
    let mut rank = 0u32;
    for (seen, (idx, o)) in ranked.into_iter().enumerate() {
        if prev_odds != Some(o) {
            rank = seen as u32 + 1; // 競争順位: 異なる値は「これまでの件数+1」へ飛ぶ。
            prev_odds = Some(o);
        }
        out[idx] = Some(rank);
    }
    out
}

/// Detect terminating-status keywords inside a horse chunk.
fn chunk_status(chunk: &[String]) -> ResultStatus {
    for line in chunk {
        if line.contains("競走除外") {
            return ResultStatus::Scratched;
        }
        if line.contains("出走取消") {
            return ResultStatus::Cancelled;
        }
        if line.contains("競走中止") {
            return ResultStatus::DidNotFinish;
        }
    }
    ResultStatus::Finished
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn chunk_status_finished_by_default() {
        let chunk = vec![
            s("5 9"),
            s("ロードトライデント牡3栗"),
            s("B 478－61：11．6"),
        ];
        assert_eq!(chunk_status(&chunk), ResultStatus::Finished);
    }

    #[test]
    fn chunk_status_scratched_keyword() {
        let chunk = vec![
            s("7 13"),
            s("マーゴットリック牡3黒鹿"),
            s("490－6 （競走除外）"),
        ];
        assert_eq!(chunk_status(&chunk), ResultStatus::Scratched);
    }

    #[test]
    fn chunk_status_cancelled_keyword() {
        let chunk = vec![s("1 1"), s("テスト馬"), s("（出走取消）")];
        assert_eq!(chunk_status(&chunk), ResultStatus::Cancelled);
    }

    #[test]
    fn chunk_status_did_not_finish_keyword() {
        let chunk = vec![s("2 3"), s("テスト馬"), s("（競走中止）")];
        assert_eq!(chunk_status(&chunk), ResultStatus::DidNotFinish);
    }

    #[test]
    fn popularity_ranks_by_ascending_odds() {
        // odds 1.7,3.0,2.6 → 人気 1,3,2（昇順順位）。
        let r = popularity_ranks(&[Some(1.7), Some(3.0), Some(2.6)]);
        assert_eq!(r, vec![Some(1), Some(3), Some(2)]);
    }

    #[test]
    fn popularity_ranks_ties_share_rank_competition_style() {
        // 同オッズは同順位、その次は件数分飛ぶ（1,2,2,4）。
        let r = popularity_ranks(&[Some(1.5), Some(2.0), Some(2.0), Some(9.9)]);
        assert_eq!(r, vec![Some(1), Some(2), Some(2), Some(4)]);
    }

    #[test]
    fn popularity_ranks_skips_missing_odds() {
        // オッズ未取得（取消等）は None のまま、順位付けの対象外。
        let r = popularity_ranks(&[Some(5.0), None, Some(2.0)]);
        assert_eq!(r, vec![Some(2), None, Some(1)]);
    }
}
