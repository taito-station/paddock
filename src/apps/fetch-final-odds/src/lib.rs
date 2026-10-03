//! 確定オッズ（馬連・ワイド・三連複）の遡及取得（#721）。
//!
//! netkeiba のオッズ API は確定後も最終オッズを返す（ADR 0048）。過去レースの確定オッズを
//! `race_odds`（直前取得値の上書き保存）とは別に、**生の応答ファイル**として保存し、
//! 解析用の TSV をオフラインで作る。DB には書かない（ADR 0068 の「払戻の専用テーブルは YAGNI」と
//! 衝突させない・研究用データの置き場はファイル）。
//!
//! - `targets`: 対象レース一覧（paddock race_id）を読み、netkeiba race_id に変換する。
//! - `fetch`: 取得ループ。取得済みはスキップ（再開可能）、取得失敗・非確定の連続で即停止。
//! - `emit`: 保存済みの生 JSON を既存パーサで読み、番兵を除いて TSV にする（ネットワーク不使用）。
//!
//! `targets` と `MIN_INTERVAL_MS`・`validate_interval`・`ensure_outside_repository` は `fill-results`（#742）も再利用している
//! 公開 API。変えるときは `fill-results` のビルドとテストも通す。

pub mod emit;
pub mod fetch;
pub mod targets;

/// 取得する券種と netkeiba の `type` 番号（馬連 4・ワイド 5・三連複 7）。
pub const ODDS_TYPES: [(u8, paddock_domain::BetType); 3] = [
    (4, paddock_domain::BetType::Quinella),
    (5, paddock_domain::BetType::Wide),
    (7, paddock_domain::BetType::Trio),
];

/// バルク取得の最小間隔（ミリ秒）。issue #721 の要件「`--max-rps 0.3` 相当」＝ 1 件 3.33 秒以上。
/// 待ちはリクエスト前の sleep なので、実際の間隔は「これ＋応答時間」になり 0.3 rps を超えない。
/// 1〜2 秒の連打はしない（ユーザー明言・IP ブロックが最重要リスク）。
pub const MIN_INTERVAL_MS: u64 = 3334;

/// 確定でない応答がこの回数続いたら止める（叩きすぎで絞られている兆候）。CLI からは変えさせない。
pub const MAX_NON_RESULT_STREAK: usize = 3;

/// 間隔の下限（max-rps 0.3 相当）を検証する。
pub fn validate_interval(interval_ms: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        interval_ms >= MIN_INTERVAL_MS,
        "--interval-ms は {MIN_INTERVAL_MS} 以上にしてください（max-rps 0.3 相当。バルク取得で連打しない）"
    );
    Ok(())
}

/// 出力先を作り、実パス（シンボリックリンクを解決したもの）がリポジトリ（祖先に `.git` がある場所）の外にあることを
/// 確かめて返す。`<out>/raw` がある場合はその実パスも確かめる。netkeiba の生の応答と派生の TSV を公開リポジトリに
/// 混ぜないため（取得元のデータを再配布しない）。ホーム自体を git 管理している環境では、ホーム配下はすべて拒否される。
pub fn ensure_outside_repository(out_dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let real = std::fs::canonicalize(out_dir)?;
    let mut checked = vec![real.clone()];
    let raw = out_dir.join("raw");
    if raw.exists() {
        checked.push(std::fs::canonicalize(raw)?);
    }
    for p in &checked {
        if let Some(repo) = p.ancestors().find(|a| a.join(".git").exists()) {
            anyhow::bail!(
                "出力先 {} がリポジトリ（{}）の中です。取得データを公開リポジトリに混ぜないよう、外（例: ~/paddock-backups/pinned/）を指定してください",
                p.display(),
                repo.display()
            );
        }
    }
    Ok(real)
}

/// 取得の結果から終了のしかたを決める（`Ok` は完了のメッセージ・`Err` は非ゼロ終了の理由）。
/// 上限未満で散発した非確定は保存していない（TSV から欠ける）ので、完了とは言わない。
pub fn exit_outcome(summary: &fetch::Summary, stop: &fetch::StopReason) -> Result<String, String> {
    match stop {
        fetch::StopReason::Completed if !summary.non_result.is_empty() => Err(format!(
            "未完了: 確定でない応答が {} 件あり保存していません。時間を空けて同じコマンドで再実行すると取り直します",
            summary.non_result.len()
        )),
        fetch::StopReason::Completed => Ok("完了".to_string()),
        fetch::StopReason::FetchError {
            race_id,
            odds_type,
            message,
        } => Err(format!(
            "取得失敗で停止しました（{race_id} type={odds_type}）: {message}。時間を空けて同じコマンドで再開してください"
        )),
        fetch::StopReason::NonResultStreak {
            race_id,
            odds_type,
            streak,
        } => Err(format!(
            "確定でない応答が {streak} 回続いたので停止しました（最後: {race_id} type={odds_type}）。叩きすぎで絞られている可能性があります。時間を空けて再開してください"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_below_max_rps_0_3_is_rejected() {
        assert!(validate_interval(MIN_INTERVAL_MS - 1).is_err());
        assert!(validate_interval(MIN_INTERVAL_MS).is_ok());
        // 1 件 1 / 0.3 秒以上
        assert!(MIN_INTERVAL_MS as f64 >= 1000.0 / 0.3);
    }

    #[test]
    fn out_dir_inside_a_repository_is_rejected_even_through_a_symlink() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let err = ensure_outside_repository(&repo.path().join("data/out")).unwrap_err();
        assert!(err.to_string().contains("リポジトリ"), "{err}");
        // リポジトリの外に置いたリンクがリポジトリの中を指していても拒否する
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("data/linked")).unwrap();
        let link = outside.path().join("odds");
        std::os::unix::fs::symlink(repo.path().join("data/linked"), &link).unwrap();
        assert!(ensure_outside_repository(&link).is_err());
        // <out>/raw だけがリポジトリの中を指すリンクでも拒否する
        let out = outside.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::os::unix::fs::symlink(repo.path().join("data/linked"), out.join("raw")).unwrap();
        assert!(ensure_outside_repository(&out).is_err());
        // リポジトリの外はそのまま通る
        let ok = outside.path().join("plain");
        assert!(ensure_outside_repository(&ok).is_ok());
    }

    #[test]
    fn exit_outcome_does_not_call_sporadic_non_results_complete() {
        let clean = fetch::Summary::default();
        assert_eq!(
            exit_outcome(&clean, &fetch::StopReason::Completed),
            Ok("完了".to_string())
        );
        let sporadic = fetch::Summary {
            non_result: vec![("R1".into(), 4, "NG".into())],
            ..fetch::Summary::default()
        };
        assert!(
            exit_outcome(&sporadic, &fetch::StopReason::Completed)
                .unwrap_err()
                .starts_with("未完了")
        );
        let failed = fetch::StopReason::FetchError {
            race_id: "R1".into(),
            odds_type: 5,
            message: "503".into(),
        };
        assert!(
            exit_outcome(&clean, &failed)
                .unwrap_err()
                .contains("取得失敗")
        );
    }
}
