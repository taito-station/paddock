//! 取得ループ（生の応答をファイルに保存する）。
//!
//! - 保存先は `<out>/raw/<race_id>-<type>.json`。確定（status=`result`）の応答だけを保存する。
//!   書き込みは一時ファイル → rename で行い、途中で落ちても壊れたファイルを「取得済み」と誤認しない。
//! - **再開**: 保存済みのファイルがある組はネットワークに触れずスキップする。
//! - **停止**: 取得失敗（HTTP エラー・接続リセット等）は**再送せず**（`fetch_odds_raw`）1 回で即停止する。
//!   確定でない応答（`NG` 等）は保存せず記録し、`max_non_result_streak` 回続いたら停止する
//!   （叩きすぎで絞られている可能性があるため。保存しないので再開時に取り直す）。

use std::io::Write;
use std::path::{Path, PathBuf};

use netkeiba_scraper::parse::parse_odds_meta;

use crate::ODDS_TYPES;
use crate::targets::Target;

/// オッズ API の取得口（テストで差し替える）。`Err` は取得失敗（ネットワーク・HTTP）。
pub trait OddsSource {
    fn fetch(&self, netkeiba_id: &str, odds_type: u8) -> Result<String, String>;
}

impl OddsSource for netkeiba_scraper::UreqNetkeibaScraper {
    fn fetch(&self, netkeiba_id: &str, odds_type: u8) -> Result<String, String> {
        self.fetch_odds_raw(netkeiba_id, odds_type)
            .map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// 全対象を処理した。
    Completed,
    /// 取得失敗（即停止）。
    FetchError {
        race_id: String,
        odds_type: u8,
        message: String,
    },
    /// 確定でない応答が連続した。
    NonResultStreak {
        race_id: String,
        odds_type: u8,
        streak: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub fetched: usize,
    pub skipped: usize,
    /// 確定でなかった応答（race_id・type・status）。保存しない。
    pub non_result: Vec<(String, u8, String)>,
}

pub fn raw_path(out_dir: &Path, race_id: &str, odds_type: u8) -> PathBuf {
    out_dir
        .join("raw")
        .join(format!("{race_id}-{odds_type}.json"))
}

fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// 対象を順に取得する。`progress` は 1 組ごとに呼ばれる（ログ用）。
pub fn run(
    targets: &[Target],
    source: &impl OddsSource,
    out_dir: &Path,
    max_non_result_streak: usize,
    mut progress: impl FnMut(usize, usize, &Summary),
) -> std::io::Result<(Summary, StopReason)> {
    std::fs::create_dir_all(out_dir.join("raw"))?;
    let mut summary = Summary::default();
    let mut streak = 0usize;
    let total = targets.len() * ODDS_TYPES.len();
    let mut done = 0usize;
    for t in targets {
        for (odds_type, _) in ODDS_TYPES {
            done += 1;
            let path = raw_path(out_dir, &t.race_id, odds_type);
            if path.exists() {
                summary.skipped += 1;
                continue;
            }
            let body = match source.fetch(&t.netkeiba_id, odds_type) {
                Ok(b) => b,
                Err(message) => {
                    let reason = StopReason::FetchError {
                        race_id: t.race_id.clone(),
                        odds_type,
                        message,
                    };
                    return Ok((summary, reason));
                }
            };
            let status = parse_odds_meta(&body)
                .map(|m| m.status)
                .unwrap_or_else(|e| format!("unparsable: {e}"));
            if status != "result" {
                summary
                    .non_result
                    .push((t.race_id.clone(), odds_type, status));
                streak += 1;
                if streak >= max_non_result_streak {
                    let reason = StopReason::NonResultStreak {
                        race_id: t.race_id.clone(),
                        odds_type,
                        streak,
                    };
                    return Ok((summary, reason));
                }
                continue;
            }
            streak = 0;
            write_atomic(&path, &body)?;
            summary.fetched += 1;
            progress(done, total, &summary);
        }
    }
    Ok((summary, StopReason::Completed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    type Respond = Box<dyn Fn(&str, u8) -> Result<String, String>>;

    struct Fake {
        calls: RefCell<Vec<(String, u8)>>,
        respond: Respond,
    }

    impl OddsSource for Fake {
        fn fetch(&self, id: &str, t: u8) -> Result<String, String> {
            self.calls.borrow_mut().push((id.to_string(), t));
            (self.respond)(id, t)
        }
    }

    fn fake(respond: impl Fn(&str, u8) -> Result<String, String> + 'static) -> Fake {
        Fake {
            calls: RefCell::new(Vec::new()),
            respond: Box::new(respond),
        }
    }

    fn result_body(t: u8) -> String {
        format!(
            r#"{{"status":"result","data":{{"official_datetime":"2025-01-05 15:02:03","odds":{{"{t}":{{}}}}}}}}"#
        )
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
    fn saves_result_bodies_and_resumes_without_refetching() {
        let dir = tempfile::tempdir().unwrap();
        let src = fake(|_, t| Ok(result_body(t)));
        let (s, stop) = run(&targets(2), &src, dir.path(), 3, |_, _, _| {}).unwrap();
        assert_eq!(stop, StopReason::Completed);
        assert_eq!((s.fetched, s.skipped), (6, 0));
        assert_eq!(
            std::fs::read_to_string(raw_path(dir.path(), "R1", 7)).unwrap(),
            result_body(7)
        );
        // 2 回目は全部スキップしてネットワークに触れない
        let again = fake(|_, _| panic!("取得済みを取り直してはいけない"));
        let (s2, stop2) = run(&targets(2), &again, dir.path(), 3, |_, _, _| {}).unwrap();
        assert_eq!(stop2, StopReason::Completed);
        assert_eq!((s2.fetched, s2.skipped), (0, 6));
        assert!(again.calls.borrow().is_empty());
    }

    #[test]
    fn stops_immediately_on_fetch_error() {
        let dir = tempfile::tempdir().unwrap();
        let src = fake(|id, t| {
            if id == "N1" && t == 5 {
                Err("GET ...: http status: 403".into())
            } else {
                Ok(result_body(t))
            }
        });
        let (s, stop) = run(&targets(3), &src, dir.path(), 3, |_, _, _| {}).unwrap();
        assert!(
            matches!(stop, StopReason::FetchError { ref race_id, odds_type: 5, .. } if race_id == "R1")
        );
        // R0 の 3 本と R1 の馬連だけ保存し、失敗の後は 1 本も叩かない
        assert_eq!(s.fetched, 4);
        assert_eq!(src.calls.borrow().len(), 5);
        assert!(!raw_path(dir.path(), "R1", 5).exists());
    }

    #[test]
    fn non_result_is_not_saved_and_streak_stops() {
        let dir = tempfile::tempdir().unwrap();
        let src = fake(|id, t| {
            if id == "N0" {
                Ok(r#"{"status":"NG","data":""}"#.to_string())
            } else {
                Ok(result_body(t))
            }
        });
        let (s, stop) = run(&targets(2), &src, dir.path(), 3, |_, _, _| {}).unwrap();
        assert_eq!(
            stop,
            StopReason::NonResultStreak {
                race_id: "R0".into(),
                odds_type: 7,
                streak: 3
            }
        );
        assert_eq!(s.fetched, 0);
        assert_eq!(s.non_result.len(), 3);
        assert!(!raw_path(dir.path(), "R0", 4).exists()); // 保存しないので再開時に取り直す
    }

    #[test]
    fn a_result_resets_the_non_result_streak() {
        let dir = tempfile::tempdir().unwrap();
        // 2 本 NG → 確定 → 2 本 NG なら連続は 2 止まりで停止しない
        let src = fake(|id, t| {
            let ng = (id == "N0" && t != 7) || (id == "N1" && t != 4);
            Ok(if ng {
                r#"{"status":"NG"}"#.to_string()
            } else {
                result_body(t)
            })
        });
        let (s, stop) = run(&targets(2), &src, dir.path(), 3, |_, _, _| {}).unwrap();
        assert_eq!(stop, StopReason::Completed);
        assert_eq!((s.fetched, s.non_result.len()), (2, 4));
    }

    #[test]
    fn leftover_temp_file_is_not_treated_as_done() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("raw")).unwrap();
        std::fs::write(
            raw_path(dir.path(), "R0", 4).with_extension("json.tmp"),
            "partial",
        )
        .unwrap();
        let src = fake(|_, t| Ok(result_body(t)));
        let (s, _) = run(&targets(1), &src, dir.path(), 3, |_, _, _| {}).unwrap();
        assert_eq!(s.fetched, 3);
    }
}
