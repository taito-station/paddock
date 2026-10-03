//! 取得ループ（結果ページの HTML をファイルに保存する）。
//!
//! - 保存先は `<out>/raw/<race_id>.html`。書き込みは一時ファイル → rename で行い、途中で落ちても壊れたファイルを
//!   「取得済み」と誤認しない。
//! - **再開**: 保存済みのファイルがあるレースはネットワークに触れずスキップする。
//! - **停止**: 取得失敗は**再送せず**（`fetch_race_result_html_once`）1 回で即停止する。
//!   結果の表を読めないページ（未確定・エラーページ等）も保存せずに止める（書き込み側で読めないものを残さない）。

use std::io::Write;
use std::path::{Path, PathBuf};

use fetch_final_odds::targets::Target;
use netkeiba_scraper::parse::parse_race_result;

/// 結果ページの取得口（テストで差し替える）。`Err` は取得失敗（ネットワーク・HTTP）。
pub trait ResultPageSource {
    fn fetch(&self, netkeiba_id: &str) -> Result<String, String>;
}

impl ResultPageSource for netkeiba_scraper::UreqNetkeibaScraper {
    fn fetch(&self, netkeiba_id: &str) -> Result<String, String> {
        self.fetch_race_result_html_once(netkeiba_id)
            .map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// 全対象を処理した。
    Completed,
    /// 取得失敗（即停止）。
    FetchError { race_id: String, message: String },
    /// 結果の表を読めないページ（保存しない）。
    NotAResultPage { race_id: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub fetched: usize,
    pub skipped: usize,
}

pub fn html_path(out_dir: &Path, race_id: &str) -> PathBuf {
    out_dir.join("raw").join(format!("{race_id}.html"))
}

fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("html.tmp");
    // 残った一時ファイル（シンボリックリンクの可能性も含む）は辿らずに消してから新しく作る
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// 対象を順に取得する。`progress` は 1 レース保存するごとに呼ばれる（ログ用）。
pub fn run(
    targets: &[Target],
    source: &impl ResultPageSource,
    out_dir: &Path,
    mut progress: impl FnMut(usize, usize, &Summary),
) -> std::io::Result<(Summary, StopReason)> {
    std::fs::create_dir_all(out_dir.join("raw"))?;
    let mut summary = Summary::default();
    for (i, t) in targets.iter().enumerate() {
        let path = html_path(out_dir, &t.race_id);
        if path.exists() {
            summary.skipped += 1;
            continue;
        }
        let body = match source.fetch(&t.netkeiba_id) {
            Ok(b) => b,
            Err(message) => {
                let reason = StopReason::FetchError {
                    race_id: t.race_id.clone(),
                    message,
                };
                return Ok((summary, reason));
            }
        };
        if let Err(e) = parse_race_result(&body, &t.netkeiba_id) {
            let reason = StopReason::NotAResultPage {
                race_id: t.race_id.clone(),
                message: e.to_string(),
            };
            return Ok((summary, reason));
        }
        write_atomic(&path, &body)?;
        summary.fetched += 1;
        progress(i + 1, targets.len(), &summary);
    }
    Ok((summary, StopReason::Completed))
}

/// 取得の結果から終了のしかたを決める（`Ok` は完了のメッセージ・`Err` は非ゼロ終了の理由）。
pub fn exit_outcome(stop: &StopReason) -> Result<String, String> {
    match stop {
        StopReason::Completed => Ok("完了".to_string()),
        StopReason::FetchError { race_id, message } => Err(format!(
            "取得失敗で停止しました（{race_id}）: {message}。時間を空けて同じコマンドで再開してください"
        )),
        StopReason::NotAResultPage { race_id, message } => Err(format!(
            "結果の表を読めないページだったので保存せずに停止しました（{race_id}）: {message}"
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    type Respond = Box<dyn Fn(&str) -> Result<String, String>>;

    struct Fake {
        calls: RefCell<Vec<String>>,
        respond: Respond,
    }

    impl ResultPageSource for Fake {
        fn fetch(&self, id: &str) -> Result<String, String> {
            self.calls.borrow_mut().push(id.to_string());
            (self.respond)(id)
        }
    }

    fn fake(respond: impl Fn(&str) -> Result<String, String> + 'static) -> Fake {
        Fake {
            calls: RefCell::new(Vec::new()),
            respond: Box::new(respond),
        }
    }

    /// `parse_race_result` が読める最小の結果ページ（着順・馬番の組）。
    pub(crate) fn result_page(rows: &[(&str, u32)]) -> String {
        let trs: String = rows
            .iter()
            .map(|(finish, num)| {
                format!(
                    r#"<tr><td class="Result_Num">{finish}</td><td class="Num Txt_C">{num}</td></tr>"#
                )
            })
            .collect();
        format!(r#"<html><body><table id="All_Result_Table">{trs}</table></body></html>"#)
    }

    fn targets(n: usize) -> Vec<Target> {
        (0..n)
            .map(|i| Target {
                race_id: format!("R{i}"),
                netkeiba_id: format!("N{i}"),
            })
            .collect()
    }

    #[test]
    fn saves_pages_and_resumes_without_refetching() {
        let dir = tempfile::tempdir().unwrap();
        let src = fake(|_| Ok(result_page(&[("1", 1)])));
        let (s, stop) = run(&targets(2), &src, dir.path(), |_, _, _| {}).unwrap();
        assert_eq!(stop, StopReason::Completed);
        assert_eq!((s.fetched, s.skipped), (2, 0));
        assert_eq!(
            std::fs::read_to_string(html_path(dir.path(), "R1")).unwrap(),
            result_page(&[("1", 1)])
        );
        let again = fake(|_| panic!("取得済みを取り直してはいけない"));
        let (s2, stop2) = run(&targets(2), &again, dir.path(), |_, _, _| {}).unwrap();
        assert_eq!(stop2, StopReason::Completed);
        assert_eq!((s2.fetched, s2.skipped), (0, 2));
    }

    #[test]
    fn stops_immediately_on_fetch_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = fake(|id| {
            if id == "N1" {
                Err("GET ...: http status: 503".into())
            } else {
                Ok(result_page(&[("1", 1)]))
            }
        });
        let (s, stop) = run(&targets(3), &src, dir.path(), |_, _, _| {}).unwrap();
        assert!(matches!(stop, StopReason::FetchError { ref race_id, .. } if race_id == "R1"));
        assert_eq!(s.fetched, 1);
        assert_eq!(src.calls.borrow().len(), 2, "失敗の後は 1 本も叩かない");
        assert!(!html_path(dir.path(), "R1").exists());
        assert!(exit_outcome(&stop).unwrap_err().contains("取得失敗"));
    }

    #[test]
    fn unreadable_page_is_not_saved_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let src = fake(|_| Ok("<html>メンテナンス中</html>".to_string()));
        let (s, stop) = run(&targets(2), &src, dir.path(), |_, _, _| {}).unwrap();
        assert!(matches!(stop, StopReason::NotAResultPage { ref race_id, .. } if race_id == "R0"));
        assert_eq!(s.fetched, 0);
        assert_eq!(src.calls.borrow().len(), 1);
        assert!(!html_path(dir.path(), "R0").exists());
        assert!(exit_outcome(&stop).is_err());
    }

    #[test]
    fn leftover_temp_symlink_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "keep").unwrap();
        std::fs::create_dir_all(dir.path().join("raw")).unwrap();
        std::os::unix::fs::symlink(
            &victim,
            html_path(dir.path(), "R0").with_extension("html.tmp"),
        )
        .unwrap();
        let src = fake(|_| Ok(result_page(&[("1", 1)])));
        let (s, _) = run(&targets(1), &src, dir.path(), |_, _, _| {}).unwrap();
        assert_eq!(s.fetched, 1);
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "keep",
            "リンク先を書き換えない"
        );
        assert_eq!(
            std::fs::read_to_string(html_path(dir.path(), "R0")).unwrap(),
            result_page(&[("1", 1)])
        );
    }

    #[test]
    fn leftover_temp_file_is_not_treated_as_done() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("raw")).unwrap();
        std::fs::write(
            html_path(dir.path(), "R0").with_extension("html.tmp"),
            "partial",
        )
        .unwrap();
        let src = fake(|_| Ok(result_page(&[("1", 1)])));
        let (s, _) = run(&targets(1), &src, dir.path(), |_, _, _| {}).unwrap();
        assert_eq!(s.fetched, 1);
    }
}
