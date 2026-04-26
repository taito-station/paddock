use pdf_ocr::render_pdf_to_pngs;

const SAMPLE: &[u8] = include_bytes!("../../../../samples/2026-3nakayama6.pdf");

#[test]
fn renders_sample_pdf_to_at_least_one_png() {
    let rendered = render_pdf_to_pngs(SAMPLE, 100).expect("render pdf");
    assert!(
        !rendered.pages.is_empty(),
        "expected at least one rendered page"
    );
    for page in &rendered.pages {
        let meta = std::fs::metadata(&page.png_path).expect("page file exists");
        assert!(meta.len() > 0, "rendered page {} is empty", page.page_num);
    }
}
