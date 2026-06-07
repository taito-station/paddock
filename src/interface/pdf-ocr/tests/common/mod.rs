use std::io::Read;
use std::path::PathBuf;

/// 結果 PDF サンプルのローカルパス（ワークスペース root の `samples/`）。
fn sample_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../samples/2026-3nakayama6.pdf")
}

/// テスト用の結果 PDF を返す。
///
/// JRA 著作物のためリポジトリには含めない（`samples/*.pdf` は gitignore 済み）。
/// ローカルに存在すればそれを使い、無ければ JRA 公式から取得して best-effort で
/// `samples/` にキャッシュする。URL は `MeetingSpec::pdf_url` と同じ規則で安定。
pub fn sample_result_pdf() -> Vec<u8> {
    let path = sample_path();
    if let Ok(bytes) = std::fs::read(&path) {
        return bytes;
    }

    let url = "https://www.jra.go.jp/datafile/seiseki/report/2026/2026-3nakayama6.pdf";
    let resp = ureq::get(url).call().expect("fetch sample result pdf from JRA");
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .expect("read sample result pdf body");

    // 次回以降のために temp→rename で atomic にキャッシュ（失敗は無視）。
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("pdf.tmp");
    if std::fs::write(&tmp, &buf).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    buf
}
