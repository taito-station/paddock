//! ゲート通過を macOS 通知で人に届ける（#584）。
//!
//! 監視は decision-support（ADR 0055 / 0060）であり、**人間に届いて初めて機能する**。
//! predict-watch は判定を stdout に流すだけだったため、2026-08-09 は 82 スイープを
//! 完走しながら判定が 20,744 行のログに埋もれたまま開催が終わった。「監視が動いていること」と
//! 「判断材料が人に届くこと」は別問題（`docs/knowledge/monitor-loop-sleep-resilience.md`）。
//!
//! 配送は **既存 shell の `notify()` と同一機構**（osascript の `display notification`）に揃える
//! ——`scripts/predict-check/snapshot_coverage_check.sh` / `scripts/backup-db.sh` 等 4 本と同じ。
//! second source を作らないのが目的で、実利は権限にある: osascript（Script Editor）への通知許可は
//! #493 で既に付与済みで、別クレート（notify-rust 等）を入れると別 bundle の許可が要り、
//! **未許可のまま無言で鳴らない**＝#584 が問題にした「届かない」を新しい形で再生産する。
//!
//! 副作用は [`send`]（とそれをデッドライン付きで包む [`send_with_deadline`]）だけに閉じ込め、
//! 閾値解決・発火判定・本文組立・起動注記・ログ行の決定はすべて純関数にする
//! （`watch::gate_caveat_lines` / `watch::print_gate_caveat` と同じ分離）。文言と判定が
//! 単体テストで固定できることが、通知が「鳴るはずなのに鳴らない」に劣化しないための担保。

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// このプラットフォームで osascript 通知を配送できるか。macOS 以外では「有効」と宣言してから
/// 全件が未配送になるだけなので、起動注記の時点で配送できないことを言う。
pub const PLATFORM_SUPPORTED: bool = cfg!(target_os = "macos");

/// osascript の**絶対パス**。PATH 解決に頼らないのは (a) PATH 汚染で別バイナリを踏まない、
/// (b) launchd の最小 PATH（既存 plist が `EnvironmentVariables.PATH` で補っているもの）でも
/// 解決できる、の 2 点のため。macOS では `/usr/bin/osascript` が固定位置。
const OSASCRIPT: &str = "/usr/bin/osascript";

/// AppleScript 本体（`-e` 3 本）。**本文は補間せず argv（`on run {msg}`）で受け取る**ので、
/// 競走名に `"` や `\` が混ざっても壊れない。タイトルは外部入力を含まないリテラルで、既存 shell の
/// `paddock <機能>` 命名に揃えてある（`paddock snapshot` / `paddock backup` /
/// `paddock verify-backup-restore` に対して `paddock watch`）。
const NOTIFY_SCRIPT: [&str; 3] = [
    "on run {msg}",
    r#"display notification msg with title "paddock watch""#,
    "end run",
];

/// 同一レースを再通知するのに要する参考ROI の上振れ幅（+0.10 = +10pt）。
///
/// 初回は必ず通知し、以降は「前回**通知した**ときの ROI」からこの幅だけ上振れしたときのみ再通知する。
/// 同じレースは窓 40 分 / 間隔 5 分で 8 回前後 Due に入るため、抑制が無いと 1 レースで 8 連投になる。
pub const NOTIFY_ROI_RESEND_DELTA: f64 = 0.10;

/// ADR 0076 が「182R / 839 スイープで通過 0 件」と**実測した**閾値（＝ROI 100%）。
///
/// 起動注記でこの実測を引くのは `notify_roi` がこの水準以上のときだけにする。`--roi-gate` を
/// 下げた探索運用（例 0.7）でも同じ文言を出すと、**測っていない閾値について「鳴りません」と
/// 宣言する**ことになる——一次資料自身が 2026-08-09 に ≥70% の通過を 7 回観測しており、
/// 起動注記が嘘をつくと「鳴らない＝妙味なし」の誤読を潰すという本来の目的が反転する。
pub const ADR0076_MEASURED_GATE: f64 = 1.0;

/// osascript の応答を待つ上限。超えたら失敗扱いにして監視ループを先へ進める。
///
/// 5 秒の根拠は「通知 1 件の表示にこれ以上かかるなら異常」という運用側の粒度。窓 40 分 /
/// 間隔 5 分のスイープに対して十分小さく、正常系で誤って打ち切ることはない。
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// 子プロセスの終了を見に行く間隔（ブロッキングスレッド上なのでポーリングでよい）。
const SEND_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 1 スイープで評価できたレース 1 件（#584）。
///
/// **閾値判定はまだ通っていない**——`evaluate_race` は ROI を見ずに評価できた全レースを返し、
/// 通知するかどうかは [`should_notify`] / [`pick_notifications`] が決める。ここを「通過済み」と
/// 誤読して判定を素通しする改修を招かないよう、型名も `GatePass` にしていない。
///
/// 通知本文の材料だけを持つ。domain 型を持ち込まず `u32` / `String` まで剥がしてあるのは、
/// 本文組立を DB 非依存の純関数として単体テストするため。
#[derive(Debug, Clone, PartialEq)]
pub struct RaceEvaluation {
    /// 重複抑制のキー（`race_id`）。
    pub race_id: String,
    /// `watch::race_label` と同一の表示ラベル（例: `函館10R 15:35`）。**判定行の先頭と同じ文字列**に
    /// することで、通知を見てそのままログを grep して買い目に戻れる（判定行はこの後ろに
    /// G1 裏タグ `🎯裏` が付くことがあるので、完全一致ではなく前方一致で追う）。
    pub label: String,
    /// 競走名（`race_cards.race_name` 由来。未保存・平場は None）。
    pub race_name: Option<String>,
    /// 参考ROI（1.0 = 100%）。
    pub roi: f64,
    /// 軸（◎）の馬番。買い目を組成できていれば必ず入る。
    pub axis: Option<u32>,
}

/// 起動時に宣言する通知設定（#584）。[`notify_status_lines`] の引数肥大を避ける。
#[derive(Debug, Clone, Copy)]
pub struct NotifySettings {
    /// `--no-notify` の反転。
    pub enabled: bool,
    /// 解決済みの発火閾値。
    pub notify_roi: f64,
    /// `--notify-roi` が明示指定されたか（false ＝ `--roi-gate` 追従）。
    pub explicit: bool,
    /// 買い妙味ゲート（🔶 の表示閾値）。
    pub roi_gate: f64,
    /// 検証候補ゲート（🔍 の表示閾値）。発火閾値がこれを下回ると `・` 表示のレースでも鳴る。
    pub notify_gate: f64,
    /// 再通知に要する上振れ幅。
    pub delta: f64,
    /// このプラットフォームで配送できるか（[`PLATFORM_SUPPORTED`]）。テストで両方を駆動するため
    /// `cfg!` を関数内に埋めずフィールドで受ける。
    pub platform_supported: bool,
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
    // `roi < notify_roi` ではなく否定形で書く。`notify_roi` が NaN のとき前者は false になり
    // **全レースが素通りして一斉発火する**（比較は常に false）。否定形なら NaN は不成立側へ倒れ、
    // 誤設定は起動時の検証（`run` の roi_gate 検証 / `resolve_notify_roi`）が声を上げる。
    if matches!(
        roi.partial_cmp(&notify_roi),
        None | Some(std::cmp::Ordering::Less)
    ) {
        return false;
    }
    match prev {
        None => true,
        Some(p) => roi >= p + delta,
    }
}

/// 1 スイープぶんの評価結果から**通知を試みるもの**を選ぶ純関数（#584・単体テスト対象）。
///
/// `state` は読むだけで**更新しない**。抑制状態を進めるのは配送に成功したときだけで
/// （[`record_notified`]）、失敗したまま「通知済み」にすると、そのレースは +10pt 上振れするまで
/// 二度と鳴らない——一過性の配送失敗 1 回でレースごと落ちるのは、決定ログが掲げる
/// 「失敗するなら鳴りすぎる側へ倒す」と逆になる。
pub fn pick_notifications(
    state: &std::collections::HashMap<String, f64>,
    evaluations: Vec<RaceEvaluation>,
    notify_roi: f64,
    delta: f64,
) -> Vec<RaceEvaluation> {
    evaluations
        .into_iter()
        .filter(|e| should_notify(state.get(&e.race_id).copied(), e.roi, notify_roi, delta))
        .collect()
}

/// 抑制状態に「**通知できた**ときの ROI」を記録する（#584・単体テスト対象）。
///
/// 記録するのは配送に成功したものだけ。閾値未満で見送った評価や配送に失敗したものを記録すると、
/// その ROI が次回判定の基準になり、本当に届けるべきときに鳴らなくなる。
pub fn record_notified(state: &mut std::collections::HashMap<String, f64>, e: &RaceEvaluation) {
    state.insert(e.race_id.clone(), e.roi);
}

/// 通知本文を組む純関数（#584・単体テスト対象）。
///
/// issue #584 の要件どおり「レース名 / 発走時刻 / 参考ROI / 軸」の 4 項目を載せる（発走時刻は
/// `label` に含まれる）。買い目全体は載せない——通知は「見に行く合図」で、そのままの買い目は
/// ログ側の `format_portfolio` 出力が持つ。
pub fn notification_message(e: &RaceEvaluation) -> String {
    let name = e
        .race_name
        .as_deref()
        .map(|n| format!(" {n}"))
        .unwrap_or_default();
    let axis = e
        .axis
        .map(|a| a.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "{}{} ・ 参考ROI {:.1}% ・ 軸{}",
        e.label,
        name,
        e.roi * 100.0,
        axis
    )
}

/// 配送の失敗（#584）。理由文字列に加えて **デッドライン切れかどうか**を型で持つ。
///
/// 呼び出し側は「1 件でもデッドラインに掛かったらそのスイープの残りは試さない」を判断するために
/// この区別が要る。理由文字列の部分一致で判定すると、文言を直した瞬間に無言で壊れる。
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryFailure {
    pub reason: String,
    /// [`SEND_TIMEOUT`] を超えて打ち切ったか（＝配送系が詰まっている兆候）。
    pub deadline: bool,
}

impl DeliveryFailure {
    fn other(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            deadline: false,
        }
    }
}

/// 同一スイープ内で配送を見送ったときの理由（#584）。
pub const SWEEP_DELIVERY_ABORTED: &str =
    "同スイープで配送がデッドラインに掛かったため試行を見送りました";

/// 通知 1 件ぶんのログ出力（#584・単体テスト対象）。
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryReport {
    /// 必ず出す 🔔 行。**配送の成否を行に載せる**ので、後から `grep '🔔'` したときに
    /// 「鳴らそうとしたが出せなかった」を「鳴った」と読み違えない。
    pub line: String,
    /// 配送失敗の警告（出さないなら None）。
    pub warning: Option<String>,
}

/// 配送結果からログ出力を決め、失敗報告の状態を更新する純関数（#584・単体テスト対象）。
///
/// **🔔 行は配送の成否に関わらず必ず出す**——通知は表示セッション依存のベストエフォートで、
/// 一次情報はログという既存の運用注記（`deployments/launchd/README.md`）に揃えるため。
///
/// 警告は「まだ報告していない失敗」のときだけ出し、**配送が成功したら再アームする**
/// （`failure_reported` を false へ戻す）。プロセス生存期間の one-shot にすると、画面ロック等の
/// 一過性の失敗が 1 度起きただけでその日いっぱい配送不能が沈黙する——#584 が潰そうとしている
/// 「静かな失敗」を配送側に作ることになる。
pub fn delivery_report(
    message: &str,
    failure: Option<&str>,
    failure_reported: &mut bool,
) -> DeliveryReport {
    match failure {
        None => {
            *failure_reported = false;
            DeliveryReport {
                line: format!("  🔔 {message}"),
                warning: None,
            }
        }
        Some(reason) => {
            let warning = if *failure_reported {
                None
            } else {
                Some(format!(
                    "⚠ macOS 通知を出せませんでした（失敗が続く間は再掲しません。ログの 🔔行が一次情報）: {reason}"
                ))
            };
            *failure_reported = true;
            DeliveryReport {
                line: format!("  🔔(未配送) {message}"),
                warning,
            }
        }
    }
}

/// 選ばれた評価を 1 件ずつ配送し、ログ行と抑制状態の更新まで行う（#584・単体テスト対象）。
///
/// 配送そのものは `deliver` に注入する。**このループが持つ 2 つの保証をテストで固定する**のが目的:
///
/// 1. **抑制状態を進めるのは配送に成功したときだけ**。失敗したまま「通知済み」にすると、
///    そのレースは +10pt 上振れするまで二度と鳴らない。
/// 2. **1 件でもデッドラインに掛かったら、そのスイープの残りは配送を試みない**。1 件あたりの
///    上限はあってもレース数ぶん直列に積み上がるので、配送が詰まっている日はスイープ末尾が
///    分オーダーで伸びる。試さなかったぶんは記録もしないので次スイープでそのまま拾い直す。
///
/// 呼び出し側（`WatchSweeper`）に置くと `&App` / `&Cli` を抱えた構造体のメソッドになってテストが
/// 書けず、**保証を消しても緑のまま**になる（実際 2 巡目まではそうなっていた）。
pub async fn deliver_all<F, Fut>(
    state: &mut std::collections::HashMap<String, f64>,
    failure_reported: &mut bool,
    selected: Vec<RaceEvaluation>,
    deliver: F,
) -> Vec<DeliveryReport>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Option<DeliveryFailure>>,
{
    let mut out = Vec::with_capacity(selected.len());
    let mut deadline_hit = false;
    for e in selected {
        let message = notification_message(&e);
        let failure = if deadline_hit {
            Some(SWEEP_DELIVERY_ABORTED.to_string())
        } else {
            let f = deliver(message.clone()).await;
            deadline_hit |= f.as_ref().is_some_and(|f| f.deadline);
            f.map(|f| f.reason)
        };
        if failure.is_none() {
            record_notified(state, &e);
        }
        out.push(delivery_report(
            &message,
            failure.as_deref(),
            failure_reported,
        ));
    }
    out
}

/// 閾値を人が読む % 表記にする（#584・単体テスト対象）。
///
/// `{:.0}%` の四捨五入だと `--notify-roi 0.755` を「≥76%」と宣言しながら実際は 75.5% で鳴る
/// ——起動注記が嘘をつかないための規律は、桁の丸めにも同じく効かせる。整数のときは小数を出さない。
fn pct(value: f64) -> String {
    let p = value * 100.0;
    if (p - p.round()).abs() < 1e-9 {
        format!("{p:.0}")
    } else {
        format!("{p:.1}")
    }
}

/// 通知設定を起動時に 1 回だけ宣言する行を組む純関数（#584・単体テスト対象）。
///
/// **これは機能要件と同格**。無いと「鳴らない＝妙味が無かった」と「そもそも既定閾値では鳴らない」が
/// 区別できず、`monitor-loop-sleep-resilience.md` が名指しした「監視の失敗は沈黙として現れる」を
/// そのまま作る。だからこそ**この注記自身が嘘をつかないこと**が重要で、ADR 0076 の実測
/// （通過 0 件）を引くのは実際に測った閾値（[`ADR0076_MEASURED_GATE`]）以上のときだけにする。
pub fn notify_status_lines(s: &NotifySettings) -> Vec<String> {
    if !s.enabled {
        return vec![
            "── macOS 通知: 無効（--no-notify）。ゲート通過はログの 🔔行にも出ず、--notify-roi は使われません（値の検証だけは行います）。"
                .to_string(),
        ];
    }
    if !s.platform_supported {
        return vec![
            "── macOS 通知: このプラットフォームでは配送できません（osascript が無い）。通知経路ごと無効なので 🔔行も出ません——ゲート通過は判定行（🔶 / 🔍 / ・）で確認してください。"
                .to_string(),
        ];
    }
    let origin = if s.explicit {
        "--notify-roi 指定"
    } else {
        "--notify-roi 未指定＝--roi-gate 追従"
    };
    // 3 つの閾値を 1 行に並べる。取り違え（表示ゲートを下げても鳴らない / 発火ゲートを
    // 下げても 🔍 は増えない）が実害を生むので、**実際に鳴る閾値がどれか**をここで確定させる。
    let mut out = vec![format!(
        "── macOS 通知: 有効・発火は参考ROI ≥ {}%（{origin}）。表示ゲートは別物で 🔶 ≥{}% / 🔍 ≥{}%（どちらもベルは鳴らさない）。",
        pct(s.notify_roi),
        pct(s.roi_gate),
        pct(s.notify_gate)
    )];
    if s.notify_roi >= ADR0076_MEASURED_GATE {
        out.push(format!(
            "   ADR 0076 が 182R / 839 スイープで通過 0 件と測ったのは ≥{}% で、この設定はその水準以上＝実質鳴りません。実地検証は --notify-roi を下げてください（例 --notify-roi 0.5 --notify-gate 0.5）。",
            pct(ADR0076_MEASURED_GATE)
        ));
    }
    if s.notify_roi < s.notify_gate {
        out.push(format!(
            // 提案値は**切り捨て**る。四捨五入だと 0.555 → 0.56 のように発火閾値を上回る値を
            // 案内してしまい、言われたとおりに指定してもこの警告が消えない。
            "   ⚠ 表示ゲート --notify-gate（≥{}%）より低いので、ログ上 ・（低シグナル）と出るレースでもベルが鳴ります。揃えるなら --notify-gate {:.2} も指定してください。",
            pct(s.notify_gate),
            (s.notify_roi * 100.0).floor() / 100.0
        ));
    }
    out.push(format!(
        "   同一レースは前回通知時から +{:.0}pt 上振れしたときだけ再通知。配送はベストエフォート（表示セッション依存）で、一次情報はログの 🔔行。鳴っても go シグナルではありません（ADR 0079）。",
        s.delta * 100.0
    ));
    out
}

/// osascript を起動して通知を 1 件出す（副作用はこの関数と [`send_with_deadline`] だけ）。
///
/// 既存 shell の `notify()` と同一の呼び出し形にする:
/// - `on run {msg}` で **本文を argv 経由**で渡す（AppleScript の文字列補間をしない）。競走名に
///   `"` や `\` が混ざっても壊れない。title は外部入力を混ぜないリテラル固定。
/// - stdin/stdout/stderr を全て null にする（osascript が端末待ちで止まらないため）。
///
/// 失敗は呼び出し側が握る。既存 shell の `|| true` 相当の握り潰しは呼び出し側で行い、
/// ここでは事実だけ返す——「通知が出せていない」ことを人に言えるようにするため
/// （黙って鳴らないのが #584 の障害そのもの）。
fn send(message: &str) -> std::io::Result<Child> {
    let mut cmd = Command::new(OSASCRIPT);
    for line in NOTIFY_SCRIPT {
        cmd.arg("-e").arg(line);
    }
    cmd.arg("--")
        .arg(message)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// [`send`] した子プロセスを [`SEND_TIMEOUT`] まで待ち、超えたら **kill して**失敗理由を返す。
///
/// `status()`（返るまで待つ）ではなく `spawn()` ＋ ポーリングにしてあるのは、**タイムアウトが
/// 実効でなければ意味が無い**ため。`tokio::time::timeout` で `spawn_blocking` を包んでも
/// blocking タスクはキャンセルできず、内側の待ちも子プロセスも残る——ブロッキングスレッドが
/// 滞留し、`#[tokio::main]` のランタイム drop が開始済み blocking タスクを join するので
/// **監視の正常終了後にプロセスが終われなくなる**（keiba-start の二重起動ガードにも波及する）。
/// ここで刈っておけば、この関数は必ず [`SEND_TIMEOUT`] 内に返る。
///
/// 戻り値は失敗理由（成功なら `None`）。そのまま [`delivery_report`] へ渡せる形にしてある。
fn send_and_wait(message: &str) -> Option<DeliveryFailure> {
    match send(message) {
        Ok(child) => wait_with_deadline(child, SEND_TIMEOUT),
        Err(e) => Some(DeliveryFailure::other(e.to_string())),
    }
}

/// 子プロセスを期限まで待ち、超えたら kill して失敗を返す（#584・単体テスト対象）。
///
/// [`send_and_wait`] から osascript 固有の部分を剥がしてあるのは、**kill 経路をテストするため**。
/// 実際に返らないコマンド（`/bin/sleep`）を食わせて「必ず期限内に返る」「子を残さない」を
/// 検査できる。ここが壊れると監視ループが止まり、プロセスも終われなくなる。
fn wait_with_deadline(mut child: Child, timeout: Duration) -> Option<DeliveryFailure> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return None,
            Ok(Some(status)) => {
                return Some(DeliveryFailure::other(format!(
                    "osascript が異常終了しました（{status}）"
                )));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    // kill → wait で確実に刈る（wait を省くとゾンビが残る）。
                    let _ = child.kill();
                    let _ = child.wait();
                    return Some(DeliveryFailure {
                        reason: format!(
                            "osascript が {} 秒以内に応答しなかったため打ち切りました",
                            timeout.as_secs()
                        ),
                        deadline: true,
                    });
                }
                std::thread::sleep(SEND_POLL_INTERVAL);
            }
            Err(e) => return Some(DeliveryFailure::other(e.to_string())),
        }
    }
}

/// 配送をブロッキングプールへ逃がす（#584）。
///
/// 同期の待ちを async のスイープから直接行うと、osascript が返らない限り**監視ループごと止まる**。
/// しかもプロセスは生存しているので規律 2 の途切れ警告も出ない（警告は次のスイープが始まって
/// 初めて測れる）——潰したい沈黙そのものを配送側に作ることになる。デッドラインは
/// [`send_and_wait`] が内側で持つので、この関数は必ず有限時間で返る。
///
/// このプラットフォームで配送手段が無い（macOS 以外）場合は叩かずに理由を返す。
pub async fn send_with_deadline(message: &str) -> Option<DeliveryFailure> {
    // 呼び出し側（`WatchSweeper::deliver_notifications`）も同じ条件で経路ごと落とすので、
    // 本番では到達しない。それでも残すのは、この関数単体の契約を「配送できない環境では
    // 失敗を返す」で閉じておくため（将来別の呼び出し口が増えても黙って叩きに行かない）。
    if !PLATFORM_SUPPORTED {
        return Some(DeliveryFailure::other(
            "macOS 以外では osascript による通知を配送できません",
        ));
    }
    let msg = message.to_string();
    match tokio::task::spawn_blocking(move || send_and_wait(&msg)).await {
        Ok(result) => result,
        Err(e) => Some(DeliveryFailure::other(format!(
            "通知タスクが異常終了しました（{e}）"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(race_name: Option<&str>, roi: f64, axis: Option<u32>) -> RaceEvaluation {
        RaceEvaluation {
            race_id: "2026-3-hakodate-2-10R".to_string(),
            label: "函館10R 15:35".to_string(),
            race_name: race_name.map(|s| s.to_string()),
            roi,
            axis,
        }
    }

    /// 指定 race_id の評価（複数レースを跨ぐ抑制のテスト用）。
    fn eval_of(race_id: &str, roi: f64) -> RaceEvaluation {
        RaceEvaluation {
            race_id: race_id.to_string(),
            label: format!("{race_id} 15:35"),
            race_name: None,
            roi,
            axis: Some(1),
        }
    }

    fn ids(evaluations: &[RaceEvaluation]) -> Vec<&str> {
        evaluations.iter().map(|e| e.race_id.as_str()).collect()
    }

    fn settings(
        notify_roi: f64,
        explicit: bool,
        roi_gate: f64,
        notify_gate: f64,
    ) -> NotifySettings {
        NotifySettings {
            enabled: true,
            notify_roi,
            explicit,
            roi_gate,
            notify_gate,
            delta: 0.10,
            platform_supported: true,
        }
    }

    /// `pick_notifications` + `record_notified` を「配送は必ず成功する」前提で回すヘルパ。
    /// `pick_notifications` の結果を全件 `record_notified` する＝配送が常に成功する世界を再現する。
    fn sweep_all_delivered(
        state: &mut std::collections::HashMap<String, f64>,
        evaluations: Vec<RaceEvaluation>,
        notify_roi: f64,
        delta: f64,
    ) -> Vec<RaceEvaluation> {
        let picked = pick_notifications(state, evaluations, notify_roi, delta);
        for e in &picked {
            record_notified(state, e);
        }
        picked
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

    #[test]
    fn suppression_holds_across_consecutive_sweeps() {
        // #584 の要件そのもの: 同一レースが 8 スイープ連続で通過しても鳴るのは初回だけ。
        let mut state = std::collections::HashMap::new();
        let first = sweep_all_delivered(&mut state, vec![eval_of("A", 1.02)], 1.0, 0.10);
        assert_eq!(ids(&first), ["A"]);
        for roi in [1.02, 1.05, 1.09, 1.00] {
            let again = sweep_all_delivered(&mut state, vec![eval_of("A", roi)], 1.0, 0.10);
            assert!(again.is_empty(), "roi={roi} で再通知してしまった");
        }
        // +10pt 上振れ（1.02 → 1.12）で初めて再通知する。
        let risen = sweep_all_delivered(&mut state, vec![eval_of("A", 1.12)], 1.0, 0.10);
        assert_eq!(ids(&risen), ["A"]);
        // 再通知後の基準は 1.12 に更新される（1.12 + 0.10 = 1.22 未満は鳴らない）。
        let after = sweep_all_delivered(&mut state, vec![eval_of("A", 1.21)], 1.0, 0.10);
        assert!(after.is_empty());
    }

    #[test]
    fn suppression_replays_real_20260809_sweeps() {
        // 実ログ（~/Library/Logs/paddock-predict-watch-20260809.log）の新潟10R 17:20 は
        // 7 スイープ連続で通過帯に入り、参考ROI は 73.9 → 80.3 → … → 76.1 と推移した。
        // --notify-roi 0.7 で監視していたらこの 1 レースだけで 7 連投になる。+10pt 抑制なら
        // 初回 73.9 のみ（次に鳴るには 83.9 以上が要る）＝ **7 通知が 1 通知に落ちる**。
        let observed = [73.9, 80.3, 80.2, 79.9, 79.1, 79.1, 76.1];
        let mut state = std::collections::HashMap::new();
        let fired: Vec<f64> = observed
            .iter()
            .flat_map(|pct| {
                sweep_all_delivered(
                    &mut state,
                    vec![eval_of("niigata10R", pct / 100.0)],
                    0.7,
                    0.10,
                )
            })
            .map(|e| (e.roi * 1000.0).round() / 10.0)
            .collect();
        assert_eq!(fired, [73.9]);
    }

    #[test]
    fn suppression_tracks_races_independently() {
        // 抑制はレース単位。A を通知済みでも B の初回は鳴る。
        let mut state = std::collections::HashMap::new();
        sweep_all_delivered(&mut state, vec![eval_of("A", 1.0)], 1.0, 0.10);
        let out = sweep_all_delivered(
            &mut state,
            vec![eval_of("A", 1.0), eval_of("B", 1.0)],
            1.0,
            0.10,
        );
        assert_eq!(ids(&out), ["B"]);
    }

    #[test]
    fn suppression_does_not_record_skipped_evaluations() {
        // 閾値未満で素通りした ROI を基準にすると、次に本当に通過したとき鳴らなくなる。
        let mut state = std::collections::HashMap::new();
        let skipped = sweep_all_delivered(&mut state, vec![eval_of("A", 0.30)], 1.0, 0.10);
        assert!(skipped.is_empty());
        assert!(state.is_empty(), "見送った評価が抑制状態を汚している");
        let fired = sweep_all_delivered(&mut state, vec![eval_of("A", 1.00)], 1.0, 0.10);
        assert_eq!(ids(&fired), ["A"]);
    }

    #[test]
    fn failed_delivery_is_retried_next_sweep() {
        // 配送に失敗した回は `record_notified` を呼ばない＝抑制状態が進まないので、
        // 次スイープで同じ ROI のまま再送される。抑制状態を先に進めていた頃は、
        // 一過性の失敗 1 回でそのレースが +10pt 上振れするまで二度と鳴らなかった。
        let mut state = std::collections::HashMap::new();
        let picked = pick_notifications(&state, vec![eval_of("A", 1.0)], 1.0, 0.10);
        assert_eq!(ids(&picked), ["A"]); // 1 スイープ目: 選ばれる → 配送失敗したので記録しない
        assert!(state.is_empty());

        let retried = pick_notifications(&state, vec![eval_of("A", 1.0)], 1.0, 0.10);
        assert_eq!(ids(&retried), ["A"], "配送失敗ぶんが次スイープで拾われない");
        record_notified(&mut state, &retried[0]); // 2 スイープ目: 配送成功

        let after = pick_notifications(&state, vec![eval_of("A", 1.0)], 1.0, 0.10);
        assert!(after.is_empty(), "成功後は通常どおり抑制される");
    }

    #[test]
    fn pick_notifications_does_not_mutate_state() {
        // 選定は読むだけ。記録は配送成功後の `record_notified` だけが行う。
        let mut state = std::collections::HashMap::new();
        state.insert("A".to_string(), 1.0);
        let before = state.clone();
        pick_notifications(
            &state,
            vec![eval_of("A", 1.5), eval_of("B", 1.0)],
            1.0,
            0.10,
        );
        assert_eq!(state, before);
    }

    #[test]
    fn should_notify_does_not_fire_on_nan_threshold() {
        // `--roi-gate nan` 経由で notify_roi が NaN になっても全レース一斉発火しない
        // （`roi < NaN` は常に false なので、素直に書くと素通りする）。誤設定は run が弾く。
        assert!(!should_notify(None, 1.5, f64::NAN, 0.10));
        assert!(!should_notify(Some(1.0), 1.5, f64::NAN, 0.10));
    }

    #[test]
    fn notify_status_reports_unsupported_platform() {
        // macOS 以外で「有効」と宣言してから全件未配送になる、という誤解を生むログを出さない。
        let mut s = settings(0.5, true, 1.0, 0.5);
        s.platform_supported = false;
        let lines = notify_status_lines(&s);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("配送できません"));
        // 3 巡目で経路ごと落としたので 🔔 行は 1 行も出ない。出ない行を案内しない。
        assert!(!lines[0].contains("🔔(未配送)"));
        assert!(lines[0].contains("判定行"));
    }

    #[test]
    fn pct_does_not_round_a_threshold_into_a_lie() {
        // 整数はそのまま、端数は 1 桁。0.755 を「76%」と宣言して 75.5% で鳴る、を起こさない。
        assert_eq!(pct(1.0), "100");
        assert_eq!(pct(0.7), "70");
        assert_eq!(pct(0.755), "75.5");
    }

    #[test]
    fn notify_status_states_the_threshold_that_actually_fires() {
        let lines = notify_status_lines(&settings(0.755, true, 1.0, 0.755));
        assert!(lines[0].contains("≥ 75.5%"), "{}", lines[0]);
        assert!(!lines[0].contains("76%"));
    }

    #[test]
    fn notify_status_emits_no_bell_with_trailing_space_outside_the_report_line() {
        // 配送できた件数は `grep -c '^  🔔 '` で数える運用なので、注記・警告の側に
        // 「🔔 」（後ろに半角スペース）を残さない。残すとカウントが常に水増しされる。
        let mut disabled = settings(1.0, false, 1.0, 0.7);
        disabled.enabled = false;
        let mut reported = false;
        let warned = delivery_report("本文", Some("理由"), &mut reported);
        let texts: Vec<String> = notify_status_lines(&settings(0.5, true, 1.0, 0.5))
            .into_iter()
            .chain(notify_status_lines(&disabled))
            .chain(warned.warning)
            .collect();
        for t in &texts {
            assert!(!t.contains("🔔 "), "「🔔 」が残っている: {t}");
        }
        // 成功行だけがカウント対象の形になっている。
        let mut ok_reported = false;
        assert!(
            delivery_report("本文", None, &mut ok_reported)
                .line
                .starts_with("  🔔 ")
        );
    }

    #[test]
    fn notify_status_suggests_a_gate_that_actually_silences_the_warning() {
        // 提案値は切り捨て。四捨五入だと 0.555 → 0.56 と案内してしまい、その通りに指定しても
        // notify_roi < notify_gate のまま同じ警告が出続ける。
        let lines = notify_status_lines(&settings(0.555, true, 1.0, 0.7));
        let suggestion = lines
            .iter()
            .find(|l| l.contains("揃えるなら"))
            .expect("表示ゲート未満の警告が出ていない");
        assert!(suggestion.contains("--notify-gate 0.55"), "{suggestion}");
        // 提案どおりに指定すれば警告が消えることまで確かめる。
        let aligned = notify_status_lines(&settings(0.555, true, 1.0, 0.55));
        assert!(!aligned.iter().any(|l| l.contains("揃えるなら")));
    }

    /// 配送結果を台本で与えるスタブ。**実際に配送を試みた本文だけ**を記録するので、
    /// 「デッドライン後は残りを試さない」保証をコール数で検査できる。
    struct ScriptedDelivery {
        outcomes: std::cell::RefCell<std::vec::IntoIter<Option<DeliveryFailure>>>,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl ScriptedDelivery {
        fn new(outcomes: Vec<Option<DeliveryFailure>>) -> Self {
            Self {
                outcomes: std::cell::RefCell::new(outcomes.into_iter()),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn deliver(&self, message: String) -> std::future::Ready<Option<DeliveryFailure>> {
            self.calls.borrow_mut().push(message);
            std::future::ready(self.outcomes.borrow_mut().next().flatten())
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    fn deadline_failure() -> DeliveryFailure {
        DeliveryFailure {
            reason: "打ち切り".to_string(),
            deadline: true,
        }
    }

    #[tokio::test]
    async fn deliver_all_records_only_successful_deliveries() {
        // 本番経路の保証 1: 配送に成功したものだけ抑制状態を進める。
        // ここが壊れると、一過性の失敗 1 回でそのレースが +10pt まで二度と鳴らなくなる。
        let mut state = std::collections::HashMap::new();
        let mut reported = false;
        let stub = ScriptedDelivery::new(vec![None, Some(DeliveryFailure::other("失敗"))]);
        let reports = deliver_all(
            &mut state,
            &mut reported,
            vec![eval_of("A", 1.0), eval_of("B", 1.0)],
            |m| stub.deliver(m),
        )
        .await;
        assert_eq!(reports.len(), 2);
        assert!(reports[0].line.starts_with("  🔔 A"));
        assert!(reports[1].line.starts_with("  🔔(未配送) B"));
        assert_eq!(state.keys().collect::<Vec<_>>(), ["A"], "state={state:?}");
    }

    #[tokio::test]
    async fn deliver_all_retries_failed_race_on_the_next_sweep() {
        // 保証 1 の帰結: 未配送のレースは次スイープでもう一度選ばれ、成功すれば記録される。
        let mut state = std::collections::HashMap::new();
        let mut reported = false;
        let stub = ScriptedDelivery::new(vec![Some(DeliveryFailure::other("失敗"))]);
        deliver_all(&mut state, &mut reported, vec![eval_of("A", 1.0)], |m| {
            stub.deliver(m)
        })
        .await;

        let picked = pick_notifications(&state, vec![eval_of("A", 1.0)], 1.0, 0.10);
        assert_eq!(ids(&picked), ["A"], "未配送ぶんが次スイープで拾われない");
        let stub2 = ScriptedDelivery::new(vec![None]);
        deliver_all(&mut state, &mut reported, picked, |m| stub2.deliver(m)).await;
        assert!(pick_notifications(&state, vec![eval_of("A", 1.0)], 1.0, 0.10).is_empty());
    }

    #[tokio::test]
    async fn deliver_all_stops_trying_after_a_deadline() {
        // 本番経路の保証 2: 1 件でもデッドラインに掛かったら残りは配送を試みない。
        // 試さなかったぶんは記録もしないので、次スイープでそのまま拾い直せる。
        let mut state = std::collections::HashMap::new();
        let mut reported = false;
        let stub = ScriptedDelivery::new(vec![Some(deadline_failure()), None, None]);
        let reports = deliver_all(
            &mut state,
            &mut reported,
            vec![eval_of("A", 1.0), eval_of("B", 1.0), eval_of("C", 1.0)],
            |m| stub.deliver(m),
        )
        .await;
        assert_eq!(stub.call_count(), 1, "デッドライン後も配送を試している");
        assert!(reports.iter().all(|r| r.line.contains("🔔(未配送)")));
        assert!(reports[1].line.contains("B") && reports[2].line.contains("C"));
        assert!(state.is_empty(), "試していないものを記録している");
    }

    #[tokio::test]
    async fn deliver_all_warns_once_then_rearms_after_success() {
        // 失敗警告は連続する間だけ黙り、成功したら再アームする（本番経路でも同じ）。
        let mut state = std::collections::HashMap::new();
        let mut reported = false;
        let stub = ScriptedDelivery::new(vec![
            Some(DeliveryFailure::other("失敗")),
            Some(DeliveryFailure::other("失敗")),
            None,
            Some(DeliveryFailure::other("失敗")),
        ]);
        let reports = deliver_all(
            &mut state,
            &mut reported,
            vec![
                eval_of("A", 1.0),
                eval_of("B", 1.0),
                eval_of("C", 1.0),
                eval_of("D", 1.0),
            ],
            |m| stub.deliver(m),
        )
        .await;
        let warned: Vec<bool> = reports.iter().map(|r| r.warning.is_some()).collect();
        assert_eq!(warned, [true, false, false, true]);
    }

    #[test]
    fn wait_with_deadline_kills_a_hanging_child() {
        // 期限を超えた子は kill され、**必ず期限内に返る**。ここが効いていないと
        // 監視ループが止まり、ランタイム drop の join でプロセスも終われなくなる。
        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("/bin/sleep の起動に失敗");
        let started = Instant::now();
        let failure = wait_with_deadline(child, Duration::from_millis(300))
            .expect("ハングした子が失敗として返っていない");
        assert!(failure.deadline, "デッドライン切れとして型付けされていない");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "期限を大きく超えて待っている: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn wait_with_deadline_returns_success_for_a_quick_child() {
        let child = Command::new("/usr/bin/true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("/usr/bin/true の起動に失敗");
        assert_eq!(wait_with_deadline(child, Duration::from_secs(5)), None);
    }

    #[test]
    fn wait_with_deadline_reports_nonzero_exit_as_failure() {
        let child = Command::new("/usr/bin/false")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("/usr/bin/false の起動に失敗");
        let failure =
            wait_with_deadline(child, Duration::from_secs(5)).expect("失敗として返らない");
        assert!(!failure.deadline);
        assert!(failure.reason.contains("異常終了"));
    }

    #[test]
    fn notification_message_contains_all_required_fields() {
        // issue #584 の要件: レース名 / 発走時刻 / 参考ROI / 軸。
        let m = notification_message(&eval(Some("巴賞"), 1.123, Some(6)));
        assert_eq!(m, "函館10R 15:35 巴賞 ・ 参考ROI 112.3% ・ 軸6");
    }

    #[test]
    fn notification_message_falls_back_without_name_and_axis() {
        // race_name 未保存（平場）・axis 不明でも本文が壊れない。
        assert_eq!(
            notification_message(&eval(None, 1.0, None)),
            "函館10R 15:35 ・ 参考ROI 100.0% ・ 軸-"
        );
    }

    #[test]
    fn delivery_report_marks_success_and_rearms() {
        let mut reported = true; // 直前の失敗を報告済みの状態
        let r = delivery_report("本文", None, &mut reported);
        assert_eq!(r.line, "  🔔 本文");
        assert!(r.warning.is_none());
        assert!(!reported, "成功したら失敗報告を再アームする");
    }

    #[test]
    fn delivery_report_marks_undelivered_and_warns_once() {
        let mut reported = false;
        let first = delivery_report("本文", Some("理由"), &mut reported);
        // 🔔 行は必ず出るが、配送できていない事実が行に載る。
        assert_eq!(first.line, "  🔔(未配送) 本文");
        assert!(first.warning.as_deref().unwrap().contains("理由"));
        assert!(reported);
        // 同じ失敗が続く間は警告を再掲しない（🔔 行は出し続ける）。
        let second = delivery_report("本文2", Some("理由"), &mut reported);
        assert_eq!(second.line, "  🔔(未配送) 本文2");
        assert!(second.warning.is_none());
    }

    #[test]
    fn delivery_report_rewarns_after_a_success() {
        // 一過性の失敗（画面ロック等）で以降その日いっぱい沈黙しないことの担保。
        let mut reported = false;
        delivery_report("A", Some("理由"), &mut reported);
        delivery_report("B", None, &mut reported); // 復旧
        let again = delivery_report("C", Some("理由"), &mut reported);
        assert!(again.warning.is_some(), "復旧後の再失敗は改めて報告する");
    }

    #[test]
    fn notify_status_cites_adr0076_only_at_the_measured_gate() {
        // 既定（100%）では ADR 0076 の実測を引いて「鳴らない」と宣言する。
        let lines = notify_status_lines(&settings(1.0, false, 1.0, 0.7));
        assert!(lines.iter().any(|l| l.contains("ADR 0076")));
        assert!(lines.iter().any(|l| l.contains("実質鳴りません")));
        assert!(lines[0].contains("--roi-gate 追従"));
    }

    #[test]
    fn notify_status_does_not_cite_adr0076_below_the_measured_gate() {
        // --roi-gate 0.7 の探索運用。ADR 0076 が測ったのは 100% なので、70% について
        // 「通過 0 件」と宣言してはならない（一次資料は 2026-08-09 に ≥70% を 7 回観測）。
        let lines = notify_status_lines(&settings(0.7, false, 0.7, 0.7));
        assert!(
            !lines.iter().any(|l| l.contains("ADR 0076")),
            "測っていない閾値について ADR 0076 を引いている"
        );
        assert!(lines[0].contains("70%"));
    }

    #[test]
    fn notify_status_labels_explicit_threshold_as_such() {
        // --notify-roi 1.2 は roi_gate 超だが「--roi-gate と同値」ではない。
        let lines = notify_status_lines(&settings(1.2, true, 1.0, 0.7));
        assert!(lines[0].contains("--notify-roi 指定"));
        assert!(!lines[0].contains("追従"));
        // 実測ゲート以上なので ADR 0076 の注記は付く。
        assert!(lines.iter().any(|l| l.contains("ADR 0076")));
    }

    #[test]
    fn notify_status_warns_when_below_display_gate() {
        // README が薦める --notify-roi 0.5 は既定 notify_gate 0.7 を下回るので、
        // ログ上 ・ と出るレースで鳴る。その食い違いを起動時に明示する。
        let lines = notify_status_lines(&settings(0.5, true, 1.0, 0.7));
        assert!(lines.iter().any(|l| l.contains("・（低シグナル）")));
        assert!(lines.iter().any(|l| l.contains("--notify-gate 0.50")));
    }

    #[test]
    fn notify_status_omits_display_gate_warning_when_aligned() {
        let lines = notify_status_lines(&settings(0.5, true, 1.0, 0.5));
        assert!(!lines.iter().any(|l| l.contains("低シグナル")));
    }

    #[test]
    fn notify_status_states_resend_rule() {
        let lines = notify_status_lines(&settings(0.5, true, 1.0, 0.5));
        assert!(lines.iter().any(|l| l.contains("+10pt")));
        assert!(lines.iter().any(|l| l.contains("🔔")));
    }

    #[test]
    fn notify_status_when_disabled() {
        let mut s = settings(1.0, false, 1.0, 0.7);
        s.enabled = false;
        let lines = notify_status_lines(&s);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("--no-notify"));
    }

    /// 実際に osascript を叩いて通知が出せることを確かめる手動テスト（macOS 表示セッションが要る）。
    ///
    /// `cargo test -p predict-watch -- --ignored --nocapture sends_real_notification`
    ///
    /// CI では表示セッションが無く落ちるので `#[ignore]`。本文に `"` と `\` を混ぜてあるのは、
    /// **argv 経由で渡す設計**（AppleScript の文字列補間をしない）が効いていることの検査を兼ねる
    /// ——補間していたらここで構文エラーになり失敗理由が返る。デッドライン付きの実経路
    /// （`send_with_deadline`）をそのまま通す。
    #[tokio::test]
    #[ignore = "macOS の表示セッションが要る手動テスト"]
    async fn sends_real_notification() {
        let msg = notification_message(&eval(Some(r#"テスト"賞\ "#), 1.234, Some(7)));
        assert_eq!(send_with_deadline(&msg).await, None);
    }
}
