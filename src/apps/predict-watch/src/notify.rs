//! ゲート通過を macOS 通知で人に届ける（#584）。
//!
//! 監視は decision-support（ADR 0055 / 0060）であり、**人間に届いて初めて機能する**。
//! predict-watch は判定を stdout に流すだけだったため、2026-08-08 / 08-09 は 82 スイープを
//! 完走しながら判定が 2 万行のログに埋もれたまま開催が終わった。「監視が動いていること」と
//! 「判断材料が人に届くこと」は別問題（`docs/knowledge/monitor-loop-sleep-resilience.md`）。
//!
//! 配送は **既存 shell の `notify()` と同一機構**（osascript の `display notification`）に揃える
//! ——`scripts/predict-check/snapshot_coverage_check.sh` / `scripts/backup-db.sh` 等 4 本と同じ。
//! second source を作らないのが目的で、実利は権限にある: osascript（Script Editor）への通知許可は
//! #493 で既に付与済みで、別クレート（notify-rust 等）を入れると別 bundle の許可が要り、
//! **未許可のまま無言で鳴らない**＝#584 が問題にした「届かない」を新しい形で再生産する。
//!
//! 副作用は [`send`] 1 本だけに閉じ込め、閾値解決・発火判定・本文組立・起動注記はすべて純関数に
//! する（`watch::gate_caveat_lines` / `watch::print_gate_caveat` と同じ分離）。文言と判定が
//! 単体テストで固定できることが、通知が「鳴るはずなのに鳴らない」に劣化しないための担保。

use std::process::{Command, ExitStatus, Stdio};

/// 通知のタイトル。既存 shell の `paddock <機能>` 命名に揃える
/// （`paddock snapshot` / `paddock backup` / `paddock verify-backup-restore`）。
const NOTIFY_TITLE: &str = "paddock watch";

/// 同一レースを再通知するのに要する参考ROI の上振れ幅（+0.10 = +10pt）。
///
/// 初回は必ず通知し、以降は「前回**通知した**ときの ROI」からこの幅だけ上振れしたときのみ再通知する。
/// 同じレースは窓 40 分 / 間隔 5 分で 8 回前後 Due に入るため、抑制が無いと 1 レースで 8 連投になる。
pub const NOTIFY_ROI_RESEND_DELTA: f64 = 0.10;

/// 通知閾値を通過したレース 1 件（#584）。
///
/// 通知本文の材料だけを持つ。domain 型を持ち込まず `u32` / `String` まで剥がしてあるのは、
/// 本文組立を DB 非依存の純関数として単体テストするため。
#[derive(Debug, Clone, PartialEq)]
pub struct GatePass {
    /// 重複抑制のキー（`race_id`）。
    pub race_id: String,
    /// `watch::race_label` と同一の表示ラベル（例: `函館10R 15:35`）。**ログ行と同じ文字列**に
    /// することで、通知を見てそのままログを grep して買い目に戻れる。
    pub label: String,
    /// 競走名（`race_cards.race_name` 由来。未保存・平場は None）。
    pub race_name: Option<String>,
    /// 参考ROI（1.0 = 100%）。
    pub roi: f64,
    /// 軸（◎）の馬番。買い目を組成できていれば必ず入る。
    pub axis: Option<u32>,
}

/// macOS 通知の発火閾値を解決する純関数（#584・単体テスト対象）。
///
/// - 未指定 → `roi_gate`（＝ issue #584 の「ゲート通過を通知する」の字面）。
/// - 明示指定 → その値。**`roi_gate` との大小は制約しない**。`resolve_notify_gate` が
///   `> roi_gate` を弾くのは「🔍 帯が構造的に空になる」という誤用があるからで、発火閾値には
///   その構造が無い。下げるのが本来の用途（#571 のゲート較正が済むまでの実地検証）で、
///   上げるのも「🔶 より厳しい通知だけ受ける」として筋が通る。
/// - 非有限（NaN/∞）・負値は弾く。NaN は比較が常に false になり、**通知が無言で鳴らなくなる**
///   ——#584 が問題にした沈黙そのものなので、誤設定として起動時に落とす。
pub fn resolve_notify_roi(explicit: Option<f64>, roi_gate: f64) -> anyhow::Result<f64> {
    match explicit {
        Some(v) if !v.is_finite() || v < 0.0 => anyhow::bail!(
            "--notify-roi（{v}）は 0 以上の有限値で指定してください（NaN は比較が常に false になり、通知が無言で鳴らなくなります）。"
        ),
        Some(v) => Ok(v),
        None => Ok(roi_gate),
    }
}

/// このレースを今スイープで通知するかを決める純関数（#584・単体テスト対象）。
///
/// - `roi` が非有限 → 通知しない（NaN の比較は常に false なので明示的に潰す）。
/// - `roi < notify_roi` → 通知しない（閾値未満はいかなる場合も鳴らさない）。
/// - `prev` が無い（このレース初回）→ 通知する。
/// - `prev` がある → 前回通知時から `delta` 以上**上振れ**したときだけ再通知する。
///
/// 帰結として、一度 1.05 で通知したレースが 0.4 へ落ち再び 1.06 へ戻っても鳴らない（次は 1.15 以上が要る）。
/// 「上振れしたときだけ再通知」の定義そのもので、剥がれたことはログで見る。
pub fn should_notify(prev: Option<f64>, roi: f64, notify_roi: f64, delta: f64) -> bool {
    if !roi.is_finite() {
        return false;
    }
    if roi < notify_roi {
        return false;
    }
    match prev {
        None => true,
        Some(p) => roi >= p + delta,
    }
}

/// 1 スイープぶんの評価結果から**実際に通知するもの**を選び、抑制状態を更新する（#584・単体テスト対象）。
///
/// 副作用は `state` の更新だけで I/O を持たないため、スイープを跨いだ抑制の挙動
/// （連続通過での連投抑止・上振れでの再通知）をそのままテストできる。記録するのは
/// 「**通知した**ときの ROI」で、見送った pass は state を汚さない——汚すと閾値未満で
/// 素通りした ROI が基準になり、次に本当に通過したとき鳴らなくなる。
pub fn select_notifications(
    state: &mut std::collections::HashMap<String, f64>,
    passes: Vec<GatePass>,
    notify_roi: f64,
    delta: f64,
) -> Vec<GatePass> {
    let mut out = Vec::new();
    for p in passes {
        if !should_notify(state.get(&p.race_id).copied(), p.roi, notify_roi, delta) {
            continue;
        }
        state.insert(p.race_id.clone(), p.roi);
        out.push(p);
    }
    out
}

/// 通知本文を組む純関数（#584・単体テスト対象）。
///
/// issue #584 の要件どおり「レース名 / 発走時刻 / 参考ROI / 軸」の 4 項目を載せる（発走時刻は
/// `label` に含まれる）。買い目全体は載せない——通知は「見に行く合図」で、そのままの買い目は
/// ログ側の `format_portfolio` 出力が持つ。
pub fn gate_pass_message(p: &GatePass) -> String {
    let name = p
        .race_name
        .as_deref()
        .map(|n| format!(" {n}"))
        .unwrap_or_default();
    let axis = p
        .axis
        .map(|a| a.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "{}{} ・ 参考ROI {:.0}% ・ 軸{}",
        p.label,
        name,
        p.roi * 100.0,
        axis
    )
}

/// 通知設定を起動時に 1 回だけ宣言する行を組む純関数（#584・単体テスト対象）。
///
/// **これは機能要件と同格**。無いと「鳴らない＝妙味が無かった」と「そもそも既定閾値では鳴らない」が
/// 区別できず、`monitor-loop-sleep-resilience.md` が名指しした「監視の失敗は沈黙として現れる」を
/// そのまま作る。既定の `notify_roi == roi_gate == 1.0` は ADR 0076 が 182R / 839 スイープを
/// 確定払戻で精算して**通過 0 件**と測った閾値なので、既定では鳴らないことを毎回明示する。
pub fn notify_status_lines(
    enabled: bool,
    notify_roi: f64,
    roi_gate: f64,
    delta: f64,
) -> Vec<String> {
    if !enabled {
        return vec![
            "── macOS 通知: 無効（--no-notify）。ゲート通過はログの 🔔 行にも出ません。"
                .to_string(),
        ];
    }
    let head = if notify_roi >= roi_gate {
        format!(
            "── macOS 通知: 有効・参考ROI ≥ {:.0}%（--roi-gate と同値）。この閾値は 182R / 839 スイープで通過 0 件（ADR 0076）＝既定では鳴りません。実地検証は --notify-roi を下げてください（例 --notify-roi 0.5）。",
            notify_roi * 100.0
        )
    } else {
        format!(
            "── macOS 通知: 有効・参考ROI ≥ {:.0}%（--notify-roi 指定・--roi-gate {:.0}% より下げた実地検証設定）。",
            notify_roi * 100.0,
            roi_gate * 100.0
        )
    };
    vec![
        head,
        format!(
            "   同一レースは前回通知時から +{:.0}pt 上振れしたときだけ再通知。通知は表示セッション依存のベストエフォートで、一次情報はログの 🔔 行です。",
            delta * 100.0
        ),
        "   通知は表示ゲート --notify-gate（🔍 マーク）とは別物で、鳴っても go シグナルではありません（ADR 0079）。".to_string(),
    ]
}

/// macOS 通知を 1 件発火する（このモジュール唯一の副作用）。
///
/// 既存 shell の `notify()` と同一の呼び出し形にする:
/// - `on run {msg}` で **本文を argv 経由**で渡す（AppleScript の文字列補間をしない）。競走名に
///   `"` や `\` が混ざっても壊れない。title は外部入力を混ぜないリテラル固定。
/// - stdin/stdout/stderr を全て null にする（osascript が端末待ちでスイープを止めないため）。
///
/// 失敗は呼び出し側が握る（`Err` と `!status.success()` の両方が失敗）。既存 shell の `|| true`
/// 相当の握り潰しは呼び出し側で行い、ここでは事実だけ返す——「通知が出せていない」ことを
/// 1 度だけ人に言えるようにするため（黙って鳴らないのが #584 の障害そのもの）。
pub fn send(message: &str) -> std::io::Result<ExitStatus> {
    Command::new("osascript")
        .arg("-e")
        .arg("on run {msg}")
        .arg("-e")
        .arg(format!(
            "display notification msg with title \"{NOTIFY_TITLE}\""
        ))
        .arg("-e")
        .arg("end run")
        .arg("--")
        .arg(message)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(race_name: Option<&str>, roi: f64, axis: Option<u32>) -> GatePass {
        GatePass {
            race_id: "2026-3-hakodate-2-10R".to_string(),
            label: "函館10R 15:35".to_string(),
            race_name: race_name.map(|s| s.to_string()),
            roi,
            axis,
        }
    }

    #[test]
    fn resolve_notify_roi_defaults_to_roi_gate() {
        // 未指定は --roi-gate 追従。roi_gate を下げた探索運用でも通知閾値が置いていかれない。
        assert_eq!(resolve_notify_roi(None, 1.0).unwrap(), 1.0);
        assert_eq!(resolve_notify_roi(None, 0.6).unwrap(), 0.6);
    }

    #[test]
    fn resolve_notify_roi_takes_explicit_below_gate() {
        // #571 のゲート較正が済むまでの主用途（手で下げて実地検証する）。
        assert_eq!(resolve_notify_roi(Some(0.5), 1.0).unwrap(), 0.5);
        assert_eq!(resolve_notify_roi(Some(0.0), 1.0).unwrap(), 0.0);
    }

    #[test]
    fn resolve_notify_roi_allows_above_gate() {
        // notify_gate（🔍 帯が空になる誤用を弾く）とは非対称であることの明示。
        assert_eq!(resolve_notify_roi(Some(1.2), 1.0).unwrap(), 1.2);
    }

    #[test]
    fn resolve_notify_roi_rejects_non_finite_and_negative() {
        assert!(resolve_notify_roi(Some(f64::NAN), 1.0).is_err());
        assert!(resolve_notify_roi(Some(f64::INFINITY), 1.0).is_err());
        assert!(resolve_notify_roi(Some(-0.1), 1.0).is_err());
    }

    #[test]
    fn should_notify_fires_first_time_at_threshold() {
        // 閾値ちょうどは通す / 直下は通さない（境界）。
        assert!(should_notify(None, 1.00, 1.00, 0.10));
        assert!(!should_notify(None, 0.99, 1.00, 0.10));
    }

    #[test]
    fn should_notify_suppresses_consecutive_sweeps() {
        // #584 の要件そのもの: 同一レースが連続スイープで通過し続けても連投しない。
        assert!(!should_notify(Some(1.00), 1.00, 1.00, 0.10));
        assert!(!should_notify(Some(1.00), 1.05, 1.00, 0.10));
        assert!(!should_notify(Some(1.00), 1.09, 1.00, 0.10));
    }

    #[test]
    fn should_notify_refires_on_ten_point_rise() {
        // 境界ちょうど（+10pt）で再通知する。
        assert!(should_notify(Some(1.00), 1.10, 1.00, 0.10));
        assert!(should_notify(Some(1.00), 1.50, 1.00, 0.10));
    }

    #[test]
    fn should_notify_never_fires_below_threshold() {
        // 一度通知したレースが閾値を割ったら、再上昇まで鳴らない（剥がれはログで見る）。
        assert!(!should_notify(Some(1.00), 0.50, 1.00, 0.10));
        assert!(!should_notify(None, 0.50, 1.00, 0.10));
    }

    #[test]
    fn should_notify_rejects_non_finite_roi() {
        // NaN は比較が常に false になり無言で落ちるので、判定側で明示的に潰す。
        assert!(!should_notify(None, f64::NAN, 1.00, 0.10));
        assert!(!should_notify(Some(1.00), f64::NAN, 1.00, 0.10));
    }

    /// 指定 race_id の GatePass（複数レースを跨ぐ抑制のテスト用）。
    fn pass_of(race_id: &str, roi: f64) -> GatePass {
        GatePass {
            race_id: race_id.to_string(),
            label: format!("{race_id} 15:35"),
            race_name: None,
            roi,
            axis: Some(1),
        }
    }

    fn ids(passes: &[GatePass]) -> Vec<&str> {
        passes.iter().map(|p| p.race_id.as_str()).collect()
    }

    #[test]
    fn select_notifications_suppresses_across_consecutive_sweeps() {
        // #584 の要件そのもの: 同一レースが 8 スイープ連続で通過しても鳴るのは初回だけ。
        let mut state = std::collections::HashMap::new();
        let first = select_notifications(&mut state, vec![pass_of("A", 1.02)], 1.0, 0.10);
        assert_eq!(ids(&first), ["A"]);
        for roi in [1.02, 1.05, 1.09, 1.00] {
            let again = select_notifications(&mut state, vec![pass_of("A", roi)], 1.0, 0.10);
            assert!(again.is_empty(), "roi={roi} で再通知してしまった");
        }
        // +10pt 上振れ（1.02 → 1.12）で初めて再通知する。
        let risen = select_notifications(&mut state, vec![pass_of("A", 1.12)], 1.0, 0.10);
        assert_eq!(ids(&risen), ["A"]);
        // 再通知後の基準は 1.12 に更新される（1.12 + 0.10 = 1.22 未満は鳴らない）。
        let after = select_notifications(&mut state, vec![pass_of("A", 1.21)], 1.0, 0.10);
        assert!(after.is_empty());
    }

    #[test]
    fn select_notifications_replays_real_20260809_sweeps() {
        // 実ログ（~/Library/Logs/paddock-predict-watch-20260809.log）の新潟10R 17:20 は
        // 7 スイープ連続で通過帯に入り、参考ROI は 73.9 → 80.3 → … → 76.1 と推移した。
        // --notify-roi 0.7 で監視していたらこの 1 レースだけで 7 連投になる。+10pt 抑制なら
        // 初回 73.9 のみ（次に鳴るには 83.9 以上が要る）＝ **7 通知が 1 通知に落ちる**。
        let observed = [73.9, 80.3, 80.2, 79.9, 79.1, 79.1, 76.1];
        let mut state = std::collections::HashMap::new();
        let fired: Vec<f64> = observed
            .iter()
            .flat_map(|pct| {
                select_notifications(
                    &mut state,
                    vec![pass_of("niigata10R", pct / 100.0)],
                    0.7,
                    0.10,
                )
            })
            .map(|p| (p.roi * 1000.0).round() / 10.0)
            .collect();
        assert_eq!(fired, [73.9]);
    }

    #[test]
    fn select_notifications_tracks_races_independently() {
        // 抑制はレース単位。A を通知済みでも B の初回は鳴る。
        let mut state = std::collections::HashMap::new();
        select_notifications(&mut state, vec![pass_of("A", 1.0)], 1.0, 0.10);
        let out = select_notifications(
            &mut state,
            vec![pass_of("A", 1.0), pass_of("B", 1.0)],
            1.0,
            0.10,
        );
        assert_eq!(ids(&out), ["B"]);
    }

    #[test]
    fn select_notifications_does_not_record_skipped_passes() {
        // 閾値未満で素通りした ROI を基準にすると、次に本当に通過したとき鳴らなくなる。
        let mut state = std::collections::HashMap::new();
        let skipped = select_notifications(&mut state, vec![pass_of("A", 0.30)], 1.0, 0.10);
        assert!(skipped.is_empty());
        assert!(state.is_empty(), "見送った pass が抑制状態を汚している");
        let fired = select_notifications(&mut state, vec![pass_of("A", 1.00)], 1.0, 0.10);
        assert_eq!(ids(&fired), ["A"]);
    }

    #[test]
    fn gate_pass_message_contains_all_required_fields() {
        // issue #584 の要件: レース名 / 発走時刻 / 参考ROI / 軸。
        let m = gate_pass_message(&pass(Some("巴賞"), 1.123, Some(6)));
        assert_eq!(m, "函館10R 15:35 巴賞 ・ 参考ROI 112% ・ 軸6");
    }

    #[test]
    fn gate_pass_message_falls_back_without_name_and_axis() {
        // race_name 未保存（平場）・axis 不明でも本文が壊れない。
        assert_eq!(
            gate_pass_message(&pass(None, 1.0, None)),
            "函館10R 15:35 ・ 参考ROI 100% ・ 軸-"
        );
    }

    #[test]
    fn notify_status_line_states_default_is_unreachable() {
        // 既定（notify_roi == roi_gate）では鳴らないことを起動時に宣言する。
        let lines = notify_status_lines(true, 1.0, 1.0, 0.10);
        assert!(lines[0].contains("ADR 0076"));
        assert!(lines[0].contains("鳴りません"));
        assert!(lines[0].contains("--notify-roi"));
    }

    #[test]
    fn notify_status_line_reports_lowered_threshold() {
        let lines = notify_status_lines(true, 0.5, 1.0, 0.10);
        assert!(lines[0].contains("50%"));
        assert!(lines[0].contains("実地検証"));
        assert!(!lines[0].contains("ADR 0076"));
    }

    #[test]
    fn notify_status_line_states_resend_rule() {
        let lines = notify_status_lines(true, 0.5, 1.0, 0.10);
        assert!(lines.iter().any(|l| l.contains("+10pt")));
        assert!(lines.iter().any(|l| l.contains("🔔")));
    }

    #[test]
    fn notify_status_line_when_disabled() {
        let lines = notify_status_lines(false, 1.0, 1.0, 0.10);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("--no-notify"));
    }

    /// 実際に osascript を叩いて通知が出せることを確かめる手動テスト（macOS 表示セッションが要る）。
    ///
    /// `cargo test -p predict-watch -- --ignored --nocapture sends_real_notification`
    ///
    /// CI では表示セッションが無く落ちるので `#[ignore]`。本文に `"` と `\` を混ぜてあるのは、
    /// **argv 経由で渡す設計**（AppleScript の文字列補間をしない）が効いていることの検査を兼ねる
    /// ——補間していたらここで構文エラーになり status が非 0 になる。
    #[test]
    #[ignore = "macOS の表示セッションが要る手動テスト"]
    fn sends_real_notification() {
        let msg = gate_pass_message(&pass(Some(r#"テスト"賞\ "#), 1.234, Some(7)));
        let status = send(&msg).expect("osascript の起動に失敗");
        assert!(status.success(), "osascript が異常終了: {status}");
    }
}
