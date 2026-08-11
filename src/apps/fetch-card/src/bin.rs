mod cli;
mod setup;

use std::process::ExitCode;

use clap::Parser;

/// degraded（単複だけ未取得・要再取得）を表す終了コード。ハード失敗(=1)・正常(=0)と区別し、
/// 消費側（例: scripts/predict-check/refresh_ev.sh は exit≠0 を FAIL 扱いし「古いオッズ警告」を
/// 出す）が win 欠落レースだけ再取得対象として識別できるようにする（#288, ADR 0049）。
const EXIT_WIN_ODDS_DEGRADED: u8 = 3;

/// ingest のエラーの扱い。「取り込み対象外＝設計どおりのスキップ（exit 0）」と
/// 「実障害（exit 1）」を取り違えないことが本コマンドの受入条件（#586, ADR 0075）。
enum IngestFailure {
    /// 仕様として取り込み対象外（障害レース）。理由を stdout に出して正常終了する。
    Skip(String),
    /// 実障害。呼び出し側へそのまま返し anyhow 経由で exit 1 にする。
    Fail(paddock_use_case::Error),
}

/// エラーを上記 2 つに振り分ける。分類を純関数に切り出すのは、**この写像そのものが受入条件**
/// だから。DB・ネットワークを要する `main` の制御フローに埋めたままだと CI で回帰を検出できない。
fn classify_failure(err: paddock_use_case::Error) -> IngestFailure {
    match err {
        paddock_use_case::Error::Unsupported(reason) => IngestFailure::Skip(reason),
        other => IngestFailure::Fail(other),
    }
}

/// スキップ時に stdout へ出す 1 行。**行頭 `スキップ: ` は消費側が読む契約**
/// （`scripts/predict-check/README.md` の判定例・仕様書の終了コード節）。文言を変えると
/// 呼び出し側の判定が無言で壊れるため、テストで固定する。
fn skip_message(reason: &str, race_id: &str, netkeiba_id: &str) -> String {
    format!(
        "スキップ: {reason}（取り込み失敗ではありません。race_id={race_id}, netkeiba={netkeiba_id}）"
    )
}

/// 取り込みが成功したときの終了コード。degraded（単複だけ未取得）のみ非 0 にする。
/// 対象外スキップは本関数に到達しない（`classify_failure` の `Skip` 側で早期 return する）。
fn exit_code_for(win_odds_degraded: bool) -> u8 {
    if win_odds_degraded {
        EXIT_WIN_ODDS_DEGRADED
    } else {
        0
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let args = cli::Cli::parse();
    let (netkeiba_id, race_id) = args.resolve_race_id()?;

    let app = setup::build_app(args.interval).await?;
    let resp = match app
        .card
        .ingest(&netkeiba_id, race_id.clone(), args.force)
        .await
    {
        Ok(resp) => resp,
        // 取り込み対象外（障害レース）は設計どおりのスキップなので、理由を stdout に出して
        // exit 0 で終える。専用 exit code を作らない理由・stderr / tracing を使わない理由は
        // ADR 0075 に一本化してある（決定を変えるときの参照先を 1 か所に保つ）。
        Err(e) => match classify_failure(e) {
            IngestFailure::Skip(reason) => {
                println!(
                    "{}",
                    skip_message(&reason, &race_id.to_string(), &netkeiba_id)
                );
                return Ok(ExitCode::SUCCESS);
            }
            IngestFailure::Fail(e) => return Err(e.into()),
        },
    };

    if resp.card_saved {
        println!(
            "出馬表: {} 頭を保存（race_id={}, netkeiba={}）",
            resp.entries_saved, race_id, netkeiba_id
        );
    } else {
        println!("出馬表: 取得済みのためスキップ（--force で再取得）");
    }
    if resp.win_odds_degraded {
        // 単複が transient 障害でリトライ後も取れず、win 欠落の部分保存を避けてオッズ未保存にした（#288）。
        // degraded の通知はここに 1 本化する。終了コードはここで断定せず末尾の return に委ねる
        // （ここは run_history より前で、history 失敗時は anyhow 経由で exit 1 になりうるため）。
        eprintln!(
            "オッズ: 単複オッズを取得できず未保存（card は保存済み）。win 欠落のため要再取得（degraded）"
        );
    } else if resp.odds_saved > 0 {
        println!(
            "オッズ: {} 件を保存（単複＋馬連・馬単・三連複・三連単）",
            resp.odds_saved
        );
    } else {
        println!("オッズ: 未確定のため保存なし");
    }

    if args.skip_history {
        println!("近走: --skip-history のため取り込みなし");
    } else {
        run_history(&app, &netkeiba_id, &resp.horse_ids).await?;
    }

    // 近走取り込み（主目的）まで終えた後で degraded を非0 exit で surface する。
    // 専用コード 3: ハード失敗(=1)と「単複だけ未取得・要再取得」を呼び出し側（例: scripts/
    // predict-check/refresh_ev.sh は exit≠0 を FAIL 扱いし「古いオッズ警告」を出す）が区別でき、
    // win 欠落レースだけ再取得を回せる（#288, ADR 0049）。`process::exit` ではなく `ExitCode` を
    // 返し、tokio ランタイム・DB プール等の Drop を走らせてから終了する。
    Ok(ExitCode::from(exit_code_for(resp.win_odds_degraded)))
}

/// 出走各馬の過去走を取り込み、予想の馬個体 factor（recent_form / horse_stats）を生かす（#103）。
/// card 取得とは独立に毎回走る（--force 不要）。
async fn run_history(
    app: &setup::App,
    netkeiba_id: &str,
    horse_ids: &[String],
) -> anyhow::Result<()> {
    // card 取得時に採れた horse_id があればそれを直接使い、同じ出馬表ページの再取得を避ける（#103）。
    // 取得済みスキップ等で horse_id が空のときのみ、race_id から出馬表を引いて horse_id を集める。
    let hist = if horse_ids.is_empty() {
        let netkeiba_ids = [netkeiba_id.to_owned()];
        app.history.fetch_and_store(&netkeiba_ids, &[]).await?
    } else {
        app.history.fetch_and_store(&[], horse_ids).await?
    };
    println!(
        "近走: {} 頭（失敗 {} 頭） / 保存: {} 近走",
        hist.horses_fetched, hist.horses_failed, hist.runs_saved
    );
    // 近走取り込みは card/オッズ（本コマンドの主目的）に対する best-effort の補完。
    // shutuba 取得が失敗しても警告のみで継続し、終了コードは 0 のままにする
    // （card/オッズ保存まで成功している実行を history 失敗で巻き添えにしない）。
    if hist.shutuba_failed > 0 {
        eprintln!(
            "警告: 出馬表 {} 件の取得に失敗（対象馬が未取得）。ログを確認してください",
            hist.shutuba_failed
        );
    }
    // 取得で horses マスタが更新された直後に pdf 成績行の horse_id を埋める（fetch-history と同じ後処理）。
    let filled = app.history.backfill_horse_ids().await?;
    println!("horse_id 紐付け: {filled} 行");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 対応外（障害レース）は Skip に振り分け、理由をそのまま持ち回る（#586）。
    #[test]
    fn unsupported_is_classified_as_skip() {
        let err = paddock_use_case::Error::Unsupported("障害レースは取り込み対象外です".into());
        match classify_failure(err) {
            IngestFailure::Skip(reason) => {
                assert_eq!(
                    reason, "障害レースは取り込み対象外です",
                    "理由は前置き無しでそのまま stdout に載る"
                );
            }
            IngestFailure::Fail(e) => panic!("対応外として扱うこと: {e}"),
        }
    }

    // 実障害は対象外に紛れさせない。ここが崩れると取り込み失敗が exit 0 で黙って通る。
    #[test]
    fn real_failures_are_not_treated_as_skip() {
        for err in [
            paddock_use_case::Error::Internal("netkeiba parse failed: boom".into()),
            paddock_use_case::Error::Fetch("connection reset".into()),
            paddock_use_case::Error::Timeout("timed out".into()),
            paddock_use_case::Error::InvalidArgument("bad race_id".into()),
            paddock_use_case::Error::NotFound("no such race".into()),
        ] {
            let label = err.to_string();
            assert!(
                matches!(classify_failure(err), IngestFailure::Fail(_)),
                "実障害はスキップ扱いにしない: {label}"
            );
        }
    }

    // 行頭 `スキップ: ` は消費側が読む契約（README の判定例・仕様書の終了コード節）。
    // 既存の「出馬表: 取得済みのためスキップ」と紛れないよう、行頭であることまで固定する。
    #[test]
    fn skip_message_starts_with_machine_readable_prefix() {
        let msg = skip_message(
            "障害レースは取り込み対象外です",
            "2026-2-chukyo-6-9R",
            "202607020609",
        );
        assert!(msg.starts_with("スキップ: "), "msg={msg}");
        assert!(msg.contains("障害レースは取り込み対象外です"), "msg={msg}");
        assert!(msg.contains("2026-2-chukyo-6-9R"), "msg={msg}");
        assert!(msg.contains("202607020609"), "msg={msg}");
    }

    // 終了コードの契約（0 / 3）。degraded だけが非 0（ADR 0049）。
    #[test]
    fn exit_code_is_zero_unless_degraded() {
        assert_eq!(exit_code_for(false), 0);
        assert_eq!(exit_code_for(true), EXIT_WIN_ODDS_DEGRADED);
    }
}
