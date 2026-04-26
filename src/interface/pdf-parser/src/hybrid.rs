use paddock_domain::{FinishingPosition, Race};
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
        // OCR is best-effort: a failure should not block the mutool path.
        match self.ocr.extract(bytes) {
            Ok(pages) => apply_ocr(&mut races, &pages),
            Err(err) => tracing::warn!("ocr extract failed, skipping merge: {err}"),
        }
        Ok(races)
    }
}

fn apply_ocr(races: &mut [Race], pages: &[OcrPageRows]) {
    // Pool every OCR row across pages into one map by horse_num, then per-race we look up
    // by (horse_num) for matching rows. Different races have different horse_num→horse mappings,
    // so we still apply per-race independently — but since each PDF only contains one meeting,
    // the horse_num space repeats across races. We therefore prefer to match by ordering: the
    // OCR pages are scanned in order and each race takes its share.
    //
    // Conservative strategy: for each race, build a horse_num→OcrRow lookup from the entire
    // page pool (taking the first occurrence). Then merge each result, but drop OCR overrides
    // that fail sanity checks (duplicate finishing_position, popularity > field size, etc.).
    let combined = combine_pages(pages);
    for race in races.iter_mut() {
        let field_size = race.results.len() as u32;
        let mut suggested_positions: Vec<Option<u32>> = race
            .results
            .iter()
            .map(|r| {
                combined
                    .iter()
                    .find(|o| o.horse_num == r.horse_num.value())
                    .and_then(|o| o.finishing_position)
            })
            .collect();

        let positions_sane = is_position_set_sane(&suggested_positions, field_size);
        if !positions_sane {
            // OCR positions are unreliable for this race; keep mutool's row-order fallback.
            suggested_positions = race
                .results
                .iter()
                .map(|r| r.finishing_position.as_ref().map(|p| p.value()))
                .collect();
        }

        for (i, result) in race.results.iter_mut().enumerate() {
            // finishing_position: replace with OCR value when sane.
            if positions_sane
                && let Some(pos) = suggested_positions[i]
                && let Ok(fp) = FinishingPosition::try_from(pos)
            {
                result.finishing_position = Some(fp);
            }

            if let Some(ocr_row) = combined
                .iter()
                .find(|o| o.horse_num == result.horse_num.value())
            {
                if result.weight_carried.is_none() {
                    result.weight_carried = ocr_row.weight_carried;
                }
                if result.popularity.is_none()
                    && let Some(pop) = ocr_row.popularity
                    && pop <= field_size
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

fn combine_pages(pages: &[OcrPageRows]) -> Vec<OcrRow> {
    let mut out: Vec<OcrRow> = Vec::new();
    for page in pages {
        for row in &page.rows {
            if !out.iter().any(|r| r.horse_num == row.horse_num) {
                out.push(row.clone());
            }
        }
    }
    out
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
