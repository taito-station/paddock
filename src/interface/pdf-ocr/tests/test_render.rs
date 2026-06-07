use pdf_ocr::render_pdf_to_pngs;

mod common;

#[test]
fn renders_sample_pdf_to_at_least_one_png() {
    let sample = common::sample_result_pdf();
    let rendered = render_pdf_to_pngs(&sample, 100).expect("render pdf");
    assert!(
        !rendered.pages.is_empty(),
        "expected at least one rendered page"
    );
    for page in &rendered.pages {
        let meta = std::fs::metadata(&page.png_path).expect("page file exists");
        assert!(meta.len() > 0, "rendered page {} is empty", page.page_num);
    }
}
