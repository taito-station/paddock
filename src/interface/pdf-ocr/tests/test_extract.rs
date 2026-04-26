use pdf_ocr::OcrExtractor;

const SAMPLE: &[u8] = include_bytes!("../../../../samples/2026-3nakayama6.pdf");

#[test]
#[ignore = "OCR is slow; run explicitly with `cargo test -p pdf-ocr --test test_extract -- --ignored --nocapture`"]
fn extracts_rows_from_each_page() {
    let extractor = OcrExtractor::default();
    let pages = extractor.extract(SAMPLE).expect("ocr extract");
    eprintln!("got {} pages", pages.len());
    for page in &pages {
        eprintln!(
            "page {}: {} rows: {:?}",
            page.page_num,
            page.rows.len(),
            page.rows
                .iter()
                .map(|r| (
                    r.horse_num,
                    r.finishing_position,
                    r.weight_carried,
                    r.popularity
                ))
                .collect::<Vec<_>>()
        );
    }
    let total_rows: usize = pages.iter().map(|p| p.rows.len()).sum();
    assert!(total_rows > 0, "expected at least some rows recovered");
}
