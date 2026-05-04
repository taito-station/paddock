use paddock_domain::{FinishingPosition, Race, ResultStatus};
use paddock_use_case::Result as UcResult;
use paddock_use_case::pdf_parser::PdfParser;
use pdf_ocr::{OcrExtractor, OcrPageRows, OcrRow};

use crate::parser::MutoolParser;

/// Combined parser: mutool text extraction provides the structural data; OCR fills in
/// fields that are lost to font issues (finishing_position, weight_carried, popularity).
pub struct HybridParser {
    pub mutool: MutoolParser,
    pub ocr: OcrExtractor,
}

impl HybridParser {
    pub fn new() -> Self {
        Self {
            mutool: MutoolParser,
            ocr: OcrExtractor::default(),
        }
    }
}

impl Default for HybridParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfParser for HybridParser {
    fn parse(&self, bytes: &[u8]) -> UcResult<Vec<Race>> {
        let mut races = self.mutool.parse(bytes)?;
        tracing::info!(
            race_count = races.len(),
            bytes = bytes.len(),
            "ocr starting"
        );
        let start = std::time::Instant::now();
        let pages = self.ocr.extract(bytes).map_err(crate::error::Error::from)?;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        tracing::info!(
            pages = pages.len(),
            elapsed_ms,
            "ocr extracted, applying merge"
        );
        apply_ocr(&mut races, &pages);
        tracing::info!("ocr merge complete");
        Ok(races)
    }
}

fn apply_ocr(races: &mut [Race], pages: &[OcrPageRows]) {
    // A PDF normally contains every race of a meeting (12 races) and each race renumbers its
    // horses 1..N. The OCR pages also span every race, so we cannot merge by horse_num alone —
    // doing so would copy race 1's row 5 onto every other race's row 5. We match by horse_name
    // (katakana, normalized) which is unique across the meeting.
    let candidates: Vec<&OcrRow> = pages
        .iter()
        .flat_map(|p| p.rows.iter())
        .filter(|r| r.horse_name.is_some())
        .collect();

    for race in races.iter_mut() {
        let field_size = race.results.len() as u32;
        let suggested_positions: Vec<Option<u32>> = race
            .results
            .iter()
            .map(|r| match_ocr_row(r.horse_name.value(), &candidates))
            .map(|ocr| ocr.and_then(|o| o.finishing_position))
            .collect();

        let positions_sane = is_position_set_sane(&suggested_positions, field_size);

        for (i, result) in race.results.iter_mut().enumerate() {
            // finishing_position: replace with OCR value when sane. Never assign to a horse that
            // didn't actually run (Scratched / Cancelled / DidNotFinish).
            if positions_sane
                && result.status == ResultStatus::Finished
                && let Some(pos) = suggested_positions[i]
                && let Ok(fp) = FinishingPosition::try_from(pos)
            {
                result.finishing_position = Some(fp);
            }

            if let Some(ocr_row) = match_ocr_row(result.horse_name.value(), &candidates) {
                if result.weight_carried.is_none()
                    && let Some(w) = ocr_row.weight_carried
                    && (48.0..=63.5).contains(&w)
                {
                    result.weight_carried = Some(w);
                }
                if result.popularity.is_none()
                    && let Some(pop) = ocr_row.popularity
                    && (1..=field_size).contains(&pop)
                {
                    result.popularity = Some(pop);
                }
                if result.trainer.is_none()
                    && let Some(tr) = &ocr_row.trainer
                    && let Ok(name) = paddock_domain::TrainerName::try_from(tr.as_str())
                {
                    result.trainer = Some(name);
                }
            }
        }
    }
}

/// Match a mutool-extracted horse name to the closest OCR row. We require that one name
/// contains the other (case-insensitive katakana substring) to tolerate small OCR errors at
/// the start/end of the name without permitting wild fuzzy matches.
fn match_ocr_row<'a>(mutool_name: &str, candidates: &[&'a OcrRow]) -> Option<&'a OcrRow> {
    let target = normalize_name(mutool_name);
    if target.is_empty() {
        return None;
    }
    candidates.iter().copied().find(|r| {
        r.horse_name
            .as_deref()
            .map(normalize_name)
            .is_some_and(|n| !n.is_empty() && (n.contains(&target) || target.contains(&n)))
    })
}

/// Normalize whitespace and replacement characters for substring matching.
fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{FFFD}')
        .collect()
}

/// A sane finishing-position set has unique values 1..=field_size with no gaps.
fn is_position_set_sane(positions: &[Option<u32>], field_size: u32) -> bool {
    let provided: Vec<u32> = positions.iter().filter_map(|p| *p).collect();
    if provided.is_empty() || provided.len() < (field_size as usize) / 2 {
        return false;
    }
    let mut sorted = provided.clone();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() != provided.len() {
        return false;
    }
    sorted.iter().all(|p| (1..=field_size).contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ocr(name: &str, num: u32, pop: Option<u32>, w: Option<f64>) -> OcrRow {
        OcrRow {
            horse_num: num,
            horse_name: Some(name.to_string()),
            finishing_position: None,
            trainer: None,
            weight_carried: w,
            popularity: pop,
        }
    }

    #[test]
    fn matches_exact_name() {
        let candidates = [ocr("ロードトライデント", 9, Some(1), Some(57.0))];
        let refs: Vec<&OcrRow> = candidates.iter().collect();
        let m = match_ocr_row("ロードトライデント", &refs).expect("should match");
        assert_eq!(m.horse_num, 9);
    }

    #[test]
    fn matches_partial_name() {
        // OCR truncated the leading letter; mutool full name still contains the OCR substring.
        let candidates = [ocr("ードトライデント", 9, Some(1), None)];
        let refs: Vec<&OcrRow> = candidates.iter().collect();
        assert!(match_ocr_row("ロードトライデント", &refs).is_some());
    }

    #[test]
    fn does_not_match_unrelated_name() {
        let candidates = [ocr("ロードトライデント", 9, Some(1), None)];
        let refs: Vec<&OcrRow> = candidates.iter().collect();
        assert!(match_ocr_row("マクローリン", &refs).is_none());
    }

    #[test]
    fn position_set_sane_complete() {
        let positions = vec![Some(1), Some(2), Some(3), Some(4)];
        assert!(is_position_set_sane(&positions, 4));
    }

    #[test]
    fn position_set_unsane_with_duplicates() {
        let positions = vec![Some(1), Some(2), Some(2), Some(4)];
        assert!(!is_position_set_sane(&positions, 4));
    }

    #[test]
    fn position_set_unsane_with_gaps_outside_range() {
        let positions = vec![Some(1), Some(2), Some(99), Some(4)];
        assert!(!is_position_set_sane(&positions, 4));
    }

    #[test]
    fn position_set_unsane_when_too_few_provided() {
        // Only 1 of 18 horses had a position read — not enough confidence.
        let positions: Vec<Option<u32>> = vec![Some(1)]
            .into_iter()
            .chain(std::iter::repeat_n(None, 17))
            .collect();
        assert!(!is_position_set_sane(&positions, 18));
    }
}
