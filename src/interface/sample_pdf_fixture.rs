//! 結果 PDF サンプルの共有テストフィクスチャ。
//!
//! 新規 crate を増やさないため、各テストクレート（`src/interface/<crate>`）から
//! `#[path = "../../sample_pdf_fixture.rs"] mod fixture;` で include して共有する。
//! すべてのクレートが同じ階層にあるため `CARGO_MANIFEST_DIR` 起点のパスは一致する。

use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// 結果 PDF サンプルを返す（プロセス内で 1 回だけ取得してキャッシュ）。
///
/// JRA 著作物のためリポジトリには含めない（`samples/*.pdf` は gitignore 済み）。
/// ローカルの `samples/` にあればそれを使い、無ければ JRA 公式からの取得を試みる。
/// 取得失敗・非 PDF 応答の場合は `None` を返す（呼び出し側はテストをスキップする）。
#[allow(dead_code)]
pub fn sample_result_pdf() -> Option<Vec<u8>> {
    static CACHE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    CACHE.get_or_init(load_result_pdf).clone()
}

fn load_result_pdf() -> Option<Vec<u8>> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../samples/2026-3nakayama6.pdf");
    if let Ok(bytes) = std::fs::read(&path) {
        return Some(bytes);
    }

    // URL は `MeetingSpec::pdf_url` と同じ規則で安定。
    let url = "https://www.jra.go.jp/datafile/seiseki/report/2026/2026-3nakayama6.pdf";
    // 無応答時にテストがハングしないようタイムアウトを設定（失敗時はスキップ）。
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut buf = Vec::new();
    match agent.get(url).call() {
        Ok(resp) => {
            if resp.into_body().into_reader().read_to_end(&mut buf).is_err() {
                eprintln!("skip: サンプル結果 PDF の読み取りに失敗");
                return None;
            }
        }
        Err(e) => {
            eprintln!("skip: サンプル結果 PDF を取得できず ({e})");
            return None;
        }
    }

    // 取得物が PDF か最低限検証し、エラーページ等の誤キャッシュ・誤使用を防ぐ。
    if !buf.starts_with(b"%PDF") {
        eprintln!("skip: 取得物が PDF ではない（先頭バイトが %PDF でない）");
        return None;
    }

    // 次回以降のため best-effort でキャッシュ（temp→rename で atomic、失敗は無視）。
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // tmp 名にプロセス ID を付けて、別テストバイナリの同時 DL でも衝突しないようにする。
    let tmp = path.with_extension(format!("pdf.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, &buf).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    Some(buf)
}
