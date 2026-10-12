use std::collections::HashMap;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc};
use monitor_loop::{RaceStatus, Sweeper, has_result, run_monitor_loop, warn_if_not_today_jst_now};
use paddock_domain::{
    HorseNum, PinnedSelection, Portfolio, RECOMMENDED_MARKET_BLEND_ALPHA, Race, RaceClass, RaceId,
    Venue, race_roughness,
};
use paddock_use_case::repository::{
    LiveEvPin, LiveEvRepository, MorningSelection, MorningSelectionRepository,
    PadPredictionRepository,
};
use paddock_use_case::{
    FixSource, MorningOutcome, compose_portfolio, recorded_axis_of, resolve_fixed_selection,
};
use predict_format::{
    PortfolioFormat, coverage_note, format_explanations, format_portfolio, format_probs,
    format_probs_with_market, format_recent_runs_warning,
};

use crate::cli::Cli;
use crate::notify::{
    ADR0076_MEASURED_GATE, NOTIFY_ROI_RESEND_DELTA, NotifySettings, PLATFORM_SUPPORTED,
    RaceEvaluation, deliver_all, notify_status_lines, pct, pick_notifications, resolve_notify_roi,
    send_with_deadline,
};
use crate::setup::App;
use crate::snapshot::{SnapshotContext, build_snapshot_record};

/// 通知（検証候補）閾値の既定値。買う閾値を下回る帯を 🔍 として残す（#345）。
const DEFAULT_NOTIFY_GATE: f64 = 0.7;

/// 通知（検証候補）閾値を解決する純関数（#345・単体テスト対象）。
///
/// - 明示指定なし → `min(roi_gate, DEFAULT_NOTIFY_GATE)`。roi_gate を既定 0.7 未満へ下げた
///   探索運用でも「notify_gate > roi_gate」で起動が壊れないようクランプする。
/// - 明示指定あり → その値。ただし非有限（NaN/∞）・負値は誤設定として弾く（NaN は比較が常に
///   false になり全レースが無言で `・` へ落ちるため）。また buy 閾値 `roi_gate` 超は 🔍 帯が
///   生じない誤用として弾く。`notify_gate == roi_gate`（境界）は許容するが、その場合 🔍 検証候補の
///   帯は構造的に空になる（`notify_gate <= roi < buy_gate` が成立しない）。等号指定＝検証候補を
///   出さない意図として通す。
pub fn resolve_notify_gate(explicit: Option<f64>, roi_gate: f64) -> anyhow::Result<f64> {
    match explicit {
        Some(ng) if !ng.is_finite() || ng < 0.0 => {
            anyhow::bail!("--notify-gate（{ng}）は 0 以上の有限値で指定してください。")
        }
        Some(ng) if ng > roi_gate => anyhow::bail!(
            "--notify-gate（{ng:.2}）は --roi-gate（{roi_gate:.2}）以下で指定してください（検証候補は買う閾値の下の帯です）。"
        ),
        Some(ng) => Ok(ng),
        None => Ok(roi_gate.min(DEFAULT_NOTIFY_GATE)),
    }
}

/// 参考 ROI を 2 つのゲートに照らして表示マークを決める純関数（#345・単体テスト対象）。
///
/// - `roi >= buy_gate` → `🔶`（買い妙味。張る/見送りの一次判断対象）
/// - `notify_gate <= roi < buy_gate` → `🔍`（検証候補。買う閾値未満だが結果照合で学ぶために残す）
/// - `roi < notify_gate` → `・`（低シグナル）
///
/// `notify_gate <= buy_gate` を前提とする（`resolve_notify_gate` が保証）。表示のみで、買う/見送りの
/// 判断や DB snapshot の verdict には影響しない（decision-support・軸ロック）。
pub fn mark_for(roi: f64, notify_gate: f64, buy_gate: f64) -> &'static str {
    debug_assert!(
        notify_gate <= buy_gate,
        "notify_gate ({notify_gate}) は buy_gate ({buy_gate}) 以下である前提"
    );
    if roi >= buy_gate {
        "🔶"
    } else if roi >= notify_gate {
        "🔍"
    } else {
        "・"
    }
}

/// 参考ROI の読み方を起動時に 1 回だけ示す注記を組む（#602 / ADR 0079・単体テスト対象）。
///
/// **🔶（買い妙味）は構造的に到達不能**——ADR 0076 が 182 レース / 839 スイープを確定払戻で精算し、
/// 通過 0 件・`Spearman(判定ROI, 実現ROI) = +0.002`（＝実現ROI に対する選別力なし）と測っている。
/// マーク自体は残す（🔍 帯は生きており結果照合の目印になる）が、**出ないものを待たせない**ために
/// 到達不能であることを明示する。閾値は動かさない——「意味のある閾値が無い」と測った以上、
/// 別の値へ動かすのは実測に反する（ADR 0040 の引き下げ棄却も ADR 0076 で支持されている）。
///
/// 文字列を返す純関数にしてあるのは、文言が実測（182R・0 件）と結びついているため
/// テストで固定するのが目的。表示は呼び出し側の `println!`。
fn gate_caveat_lines(roi_gate: f64, notify_gate: f64) -> Vec<String> {
    // ADR 0076 が「通過 0 件」と**実測した**のは ≥100%。`--roi-gate` を下げた探索運用でも同じ文言を
    // 出すと、測っていない閾値について到達不能を断言することになる（#584 で通知側に入れた規律を
    // ここにも通す。一次資料は 2026-08-09 に ≥70% の通過を 7 回観測している）。
    let mut out = vec![if roi_gate >= ADR0076_MEASURED_GATE {
        format!(
            "── 参考ROI の読み方: 🔶（≥{}%）は 182R / 839 スイープの実測で到達 0 件。判定ROI に実現ROI の選別力は無い（ADR 0076）。",
            pct(roi_gate)
        )
    } else {
        format!(
            "── 参考ROI の読み方: 🔶（≥{}%）は買う閾値の表示マーク。判定ROI に実現ROI の選別力は無い（ADR 0076。到達 0 件と実測したのは ≥{}% で、この設定はそれより低い）。",
            pct(roi_gate),
            pct(ADR0076_MEASURED_GATE)
        )
    }];
    if notify_gate < roi_gate {
        out.push(format!(
            "   🔍（≥{}%）は結果照合のための目印で、張り推奨ではない。",
            pct(notify_gate)
        ));
    }
    out.push(
        "   張る/見送りは参考ROIでは決めない——手動のハンデ精査と執行の規律（軸ロック＋ズレ増額）で決める（ADR 0055/0060）。"
            .to_string(),
    );
    out
}

fn print_gate_caveat(roi_gate: f64, notify_gate: f64) {
    for line in gate_caveat_lines(roi_gate, notify_gate) {
        println!("{line}");
    }
}

/// 当該レースが「G1 裏」（G1 開催日・別場の非重賞）かを判定する純関数（#345・単体テスト対象）。
///
/// 当日どこかで G1 が行われ、かつ当該レースが (a) 非重賞（G1/G2/G3 以外）かつ (b) その G1
/// 開催場と別場のとき true。「別場」は当日 G1 を開催する全 venue 集合との照合で判定し、G1
/// 開催場と同一場の平場は裏に含めない。クラス不明（None）は非重賞と断定できないため false。
///
/// G1 とその裏に旨みが偏在する、という未検証仮説（#345）の可視化用フラグ。買う/見送りの判断や
/// 軸には影響しない（decision-support・-EV でも通知に残すためのタグ）。
///
/// 注意: `day_classes` は各レースの race_card 由来の race_class から作る。当日 G1 の race_card が
/// 未取得、または title の parse に失敗すると day_classes に G1 が現れず、裏タグもバナーも無言で
/// 出なくなる（G1 開催日は G1 のカードも fetch-card しておくこと）。
pub fn is_g1_ura(
    venue: Venue,
    race_class: Option<RaceClass>,
    day_classes: &[(Venue, Option<RaceClass>)],
) -> bool {
    // 非重賞（クラス既知）でなければ裏対象外。
    let Some(class) = race_class else {
        return false;
    };
    if class.is_graded() {
        return false;
    }
    // 当日どこかで G1 が開催されているか。無ければ裏は成立しない。
    let g1_today = day_classes
        .iter()
        .any(|(_, c)| c.is_some_and(|c| c.is_g1()));
    if !g1_today {
        return false;
    }
    // 当該場で G1 が行われているなら「別場」ではない（同一開催場の平場は裏に含めない）。
    let g1_at_this_venue = day_classes
        .iter()
        .any(|(v, c)| *v == venue && c.is_some_and(|c| c.is_g1()));
    !g1_at_this_venue
}

/// `--race-budget-override <race_id>=<円>` の並びを `race_id → 予算(円)` マップにパースする（#342）。
/// 各要素は 1 個の `=` で分割し、左辺は `RaceId` として**不正文字を早期に弾く**（`_` 等。ただし
/// valid-format な pid 取り違えは形式検証を通るため runtime の unmatched 警告で検出する二段構え）、
/// 右辺は正の整数（円）としてパースする。重複 race_id は誤設定として弾く（後勝ちで黙って上書きしない）。
/// 予算は **100 円以上**を要求する（`build_portfolio` は券種予算を 100 円単位に floor するため、100 円
/// 未満は全券種 0 円＝空伝票になり増額用途として無意味。100 円単位への切り捨て自体は build_portfolio に
/// 委ねる）。空入力は空マップ（override なし）。純関数として単体テストする。
fn parse_race_budget_overrides(entries: &[String]) -> anyhow::Result<HashMap<String, u64>> {
    let mut out = HashMap::with_capacity(entries.len());
    for entry in entries {
        let (raw_id, raw_yen) = entry.split_once('=').ok_or_else(|| {
            anyhow::anyhow!(
                "--race-budget-override は `<race_id>=<円>` 形式で指定してください: {entry:?}"
            )
        })?;
        let race_id = RaceId::try_from(raw_id.trim()).map_err(|e| {
            anyhow::anyhow!("--race-budget-override の race_id が不正です（{raw_id:?}）: {e}")
        })?;
        let yen: u64 = raw_yen.trim().parse().map_err(|_| {
            anyhow::anyhow!(
                "--race-budget-override の予算は正の整数で指定してください: {raw_yen:?}"
            )
        })?;
        if yen < 100 {
            anyhow::bail!(
                "--race-budget-override の予算は 100 円以上で指定してください（100 円未満は空伝票になります）: {entry:?}"
            );
        }
        let key = race_id.value().to_string();
        if out.insert(key, yen).is_some() {
            anyhow::bail!(
                "--race-budget-override で同じ race_id を複数回指定しています: {}",
                race_id.value()
            );
        }
    }
    Ok(out)
}

/// 当該レースの予算（円）を解決する（#342）。override があればそのレースだけ per-race 値、無ければ
/// 既定 `--race-budget`。予算は `build_portfolio` の配分額にのみ効き、軸・点数・相手は変えない。純関数。
fn resolve_race_budget(
    overrides: &HashMap<String, u64>,
    race_id: &str,
    default_budget: u64,
) -> u64 {
    overrides.get(race_id).copied().unwrap_or(default_budget)
}

/// 1 スイープ全体で共有する評価コンテキスト（#345）。sweep / evaluate_race の引数肥大を避ける。
/// notify_gate は run で 1 度だけ解決した値、blend_alpha は解決済みの市場ブレンド重み。
/// race_budget_overrides は run で 1 度だけパースした per-race 予算（#342・空なら override なし）。
#[derive(Clone, Copy)]
struct SweepCtx<'a> {
    cli: &'a Cli,
    notify_gate: f64,
    blend_alpha: Option<f64>,
    race_budget_overrides: &'a HashMap<String, u64>,
    /// その日の初回スイープで確定した買い目選定（#601 軸ロック）。`race_id` 引き。
    /// スイープ開始時に 1 回だけ引いて全レースで使い回す（レースごとに引くと N+1 になる）。
    /// 未評価のレースは不在＝このスイープで選定が確定する。
    pins: &'a HashMap<String, LiveEvPin>,
    /// 朝の選定（#767）。`race_id` 引き。初回スイープより優先する固定の元。
    morning: &'a HashMap<String, MorningSelection>,
}

/// レースが監視窓に入る時刻（UTC）。開催日 `date` の JST `post` から `window` を引いたもの（#767）。
/// これ以降に保存された単勝は predict-watch 自身の初回スイープの保存なので「朝」と見なさない。
fn window_entry_utc(date: NaiveDate, post: NaiveTime, window: Duration) -> DateTime<Utc> {
    let jst = FixedOffset::east_opt(9 * 3600).expect("+09:00 は有効");
    let post = jst
        .from_local_datetime(&date.and_time(post))
        .single()
        .expect("固定オフセットでは一意に決まる");
    post.with_timezone(&Utc) - window
}

/// 評価のときに記録◎（pad）を読むか（#601・#767）。固定の元（初回スイープ・朝の選定）が無いときだけ読む。
/// 朝の選定は組むときに記録◎ を軸へ強制済みで、保存した後に入った記録◎ は使わない（盤と同じ規則）。
/// 初回スイープの固定がある間も読まない（その軸に反映済み・毎スイープ引くと N+1）。
fn reads_recorded_axis(has_first_sweep: bool, has_morning: bool) -> bool {
    !has_first_sweep && !has_morning
}

/// 朝の選定を計算するレース（#767）。`(窓に入ったレース, 窓の前のレース)` の添字を返す。
///
/// まだ固定の元（朝の選定・初回スイープ）が無いレースだけが対象。初回スイープで固定済みのレースに
/// 後から朝の選定を作らない（窓に入った後に固定の元を入れ替えない＝軸フリップ禁止）。
/// 発走済み・発走時刻不明は対象外。窓に入ったレースは評価の前に、窓の前のレースは評価の後に計算する
/// （初回スイープの評価を遅らせない）。
///
/// 初回スイープの固定を読めなかったスイープ（`pins_loaded == false`）では何も計算しない。固定済みかを
/// 判断できず、T-40 で固定したレースに朝の選定を後から作ってしまうため（次のスイープでやり直す）。
fn morning_targets(
    race_ids: &[&str],
    statuses: &[RaceStatus],
    pins_loaded: bool,
    is_fixed: impl Fn(&str) -> bool,
) -> (Vec<usize>, Vec<usize>) {
    let mut due = Vec::new();
    if !pins_loaded {
        return (due, Vec::new());
    }
    let mut later = Vec::new();
    for (i, (id, st)) in race_ids.iter().zip(statuses).enumerate() {
        if is_fixed(id) {
            continue;
        }
        match st {
            RaceStatus::Due => due.push(i),
            RaceStatus::NotYet => later.push(i),
            RaceStatus::Started | RaceStatus::Unknown => {}
        }
    }
    (due, later)
}

/// 1 スイープで朝の選定を計算した結果の件数（ログ用・#767）。
#[derive(Default)]
struct MorningTally {
    saved: usize,
    no_snapshot: usize,
    too_late: usize,
    not_composable: usize,
    failed: usize,
}

impl MorningTally {
    fn line(&self) -> Option<String> {
        let attempted =
            self.saved + self.no_snapshot + self.too_late + self.not_composable + self.failed;
        (attempted > 0).then(|| {
            format!(
                "   朝の選定（#767）: 確定 {} / 朝の単勝待ち {} / 窓に入った後の単勝しか無い {}（T-40 で固定）/ 組めず {} / 失敗 {}",
                self.saved, self.no_snapshot, self.too_late, self.not_composable, self.failed
            )
        })
    }
}

/// UTC rfc3339（`live_ev_snapshots.captured_at`）を JST `HH:MM` にする。解釈できなければそのまま返す。
fn jst_hhmm(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .and_then(|dt| {
            chrono::FixedOffset::east_opt(9 * 3600)
                .map(|jst| dt.with_timezone(&jst).format("%H:%M").to_string())
        })
        .unwrap_or_else(|| rfc3339.to_string())
}

fn nums(v: &[HorseNum]) -> String {
    v.iter()
        .map(|n| n.value().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// 固定の適用状況を 1 行で出す（#601・#767 で出所に朝を追加）。**固定されていないことも読めるようにする**
/// のが要点で、従来のログは選定が毎回作り直されている事実がどこにも出ていなかった。
fn print_pin_status(label: &str, pinned: &PinnedSelection, source: &FixSource) {
    if pinned.is_empty() {
        println!("  🆕 {label}: このスイープで買い目を確定（以降のスイープでは固定）");
        return;
    }
    let axis = pinned
        .axis
        .map(|a| a.value().to_string())
        .unwrap_or_else(|| "-".to_string());
    let partners = pinned.partners.as_deref().map(nums).unwrap_or_default();
    let with_recorded = |recorded: bool, base: String| {
        if recorded {
            format!("記録◎ ＋ {base}")
        } else {
            base
        }
    };
    let source = match source {
        FixSource::None => "固定なし".to_string(),
        FixSource::RecordedAxis => "記録◎".to_string(),
        FixSource::Morning {
            odds_fetched_at,
            recorded,
        } => with_recorded(*recorded, format!("朝 {}", jst_hhmm(odds_fetched_at))),
        FixSource::FirstSweep {
            captured_at,
            recorded,
        } => with_recorded(*recorded, format!("初回スイープ {}", jst_hhmm(captured_at))),
    };
    let konsen = match pinned.konsen_band.as_deref() {
        Some(b) if b.len() >= 4 => format!(" 混戦[{}]", nums(b)),
        Some(_) => " 非混戦".to_string(),
        None => String::new(),
    };
    println!("  🔒 {label}: 軸{axis} 相手{partners}{konsen} 固定（{source}）");
}

/// 固定と現在のライブ再計算との乖離を出す（#601）。**軸は動かさず、乖離だけ知らせる**
/// （CLAUDE.md 軸ロック: オッズが動いただけで ◎ を見直さない）。
fn print_pin_drift(
    label: &str,
    pinned: &PinnedSelection,
    portfolio: &Portfolio,
    blended: &[paddock_domain::HorseProbability],
) {
    // ライブ再計算の首位（＝固定しなければ軸になっていた馬）。
    let live_top = blended
        .iter()
        .max_by(|a, b| {
            a.win_prob
                .partial_cmp(&b.win_prob)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.horse_num);
    if let (Some(fixed), Some(live)) = (pinned.axis, live_top)
        && fixed != live
    {
        println!(
            "  ⚠ {label}: ライブ再計算の首位は{}（軸は固定の{}のまま。見直すのは新情報が出たときだけ）",
            live.value(),
            fixed.value()
        );
    }
    // 固定した軸が非出走（取消）なら build_portfolio がライブ首位へフォールバックしている。
    if let (Some(fixed), Some(actual)) = (pinned.axis, portfolio.axis)
        && fixed != actual
    {
        println!(
            "  ⚠ {label}: 固定軸{}が非出走のため、ライブ首位{}へフォールバックした（取消＝新情報）",
            fixed.value(),
            actual.value()
        );
    }
    // 固定した相手のうち非出走の馬は落とし、ライブ順位からの補充はしない（点数が減る）。
    if let Some(fixed) = pinned.partners.as_deref()
        && portfolio.partners.len() < fixed.len()
    {
        let dropped: Vec<String> = fixed
            .iter()
            .filter(|n| !portfolio.partners.contains(n))
            .map(|n| n.value().to_string())
            .collect();
        println!(
            "  ⚠ {label}: 固定相手{}が非出走のため除外（点数 {}→{}・補充はしない）",
            dropped.join(","),
            fixed.len(),
            portfolio.partners.len()
        );
    }
}

/// 発走時刻付きのレース 1 件（races_by_date の Race ＋ race_card の post_time / race_class / race_name）。
struct Slot {
    race: Race,
    post_time: Option<NaiveTime>,
    /// レースクラス（#345・race_card 由来）。G1 裏レース検出に使う。未取得・判定不能は None。
    race_class: Option<RaceClass>,
    /// 競走名（#584・race_card 由来。例「巴賞」）。macOS 通知の本文にだけ使う。未保存は None。
    race_name: Option<String>,
}

/// 指定日の全レースを取得し、各レースの post_time / race_class / race_name（race_card 由来）を引き当てる。
///
/// #459 で per-race `race_card`（N+1）を日付一括クエリに置き換えた。#584 で race_name を足して 3 本。
/// races_by_date の各レースに対し、一括マップから引く（マップに無い＝未保存 NULL は None＝旧 per-card
/// 経路で `card.post_time`/`card.race_class` が None だったのと同一集合になる）。
///
/// `race_name` は通知本文にしか使わないが、**`--no-notify` でも引く**。分岐を持ち込むより
/// 「slots の形はフラグに依らない」ほうが単純で、日付一括の 1 クエリ／スイープは
/// スクレイプ（`scrape_delay` × 対象レース）に対して無視できるため。
async fn load_slots(app: &App, date: chrono::NaiveDate) -> anyhow::Result<Vec<Slot>> {
    let races = app.interactor.races_by_date(date).await?;
    let post_times = app.interactor.post_times_by_date(date).await?;
    let race_classes = app.interactor.race_classes_by_date(date).await?;
    let race_names = app.interactor.race_names_by_date(date).await?;
    let mut slots = Vec::with_capacity(races.len());
    for race in races {
        let post_time = post_times.get(&race.race_id).copied();
        let race_class = race_classes.get(&race.race_id).copied();
        let race_name = race_names.get(&race.race_id).cloned();
        slots.push(Slot {
            race,
            post_time,
            race_class,
            race_name,
        });
    }
    Ok(slots)
}

/// レース表示ラベル（例: `函館10R 15:35`）。発走時刻不明なら `--:--`。
fn race_label(slot: &Slot) -> String {
    let post = slot
        .post_time
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".to_string());
    format!(
        "{}{}R {}",
        slot.race.venue.as_jp(),
        slot.race.race_num,
        post
    )
}

/// 1 スイープ: Due レースのオッズを再取得し EV/ROI を再計算、結果を 1 行ずつ出力する。
/// `statuses` は `slots` と同順の発走状態（呼び出し側で 1 度だけ算出して使い回す）。
///
/// 戻り値は**評価できた全レース**の [`RaceEvaluation`]（#584）。ここでは通知閾値の判定をしない——
/// 判定は `should_notify` 1 箇所に集約し、副作用（osascript）は `&mut self` を持つ
/// [`WatchSweeper::sweep`] 側に置く（`SweepCtx` は `Copy` で状態を持てず、`Sweeper::sweep` の
/// 戻りは `Send` 制約があるので内部可変性も使えない）。
async fn sweep(
    app: &App,
    slots: &[Slot],
    statuses: &[RaceStatus],
    now: NaiveTime,
    captured_at: &str,
    ctx: SweepCtx<'_>,
) -> Vec<RaceEvaluation> {
    let SweepCtx {
        cli, notify_gate, ..
    } = ctx;
    let due: Vec<&Slot> = slots
        .iter()
        .zip(statuses)
        .filter(|(_, st)| **st == RaceStatus::Due)
        .map(|(s, _)| s)
        .collect();
    let unknown = statuses
        .iter()
        .filter(|st| **st == RaceStatus::Unknown)
        .count();

    // notify_gate == roi_gate（🔍 帯が構造的に空）のときはヘッダから 🔍 表記を落とし、
    // 出ないマークを案内しない（表示と実挙動を一致させる）。
    let notify_part = if notify_gate < cli.roi_gate {
        format!(" ・ 🔍検証候補≥{}%", pct(notify_gate))
    } else {
        String::new()
    };
    println!(
        "── {} スイープ: 対象 {} レース（窓 {}分 / 🔶買い妙味≥{}%{}・判定は手動精査）",
        now.format("%H:%M"),
        due.len(),
        cli.window,
        pct(cli.roi_gate),
        notify_part,
    );
    if unknown > 0 {
        println!(
            "   ※ 発走時刻不明（post_time 無し）{unknown} レースは対象外。fetch-card 済みか確認。"
        );
    }

    // G1 裏レース検出用に、当日の全レース（Due 以外も含む）の (場, クラス) を集める。G1 は
    // 当該レースが Due になる時点と別の場・別の時間帯で行われうるため、Due だけでは判定できない。
    let day_classes: Vec<(Venue, Option<RaceClass>)> =
        slots.iter().map(|s| (s.race.venue, s.race_class)).collect();
    if day_classes
        .iter()
        .any(|(_, c)| c.is_some_and(|c| c.is_g1()))
    {
        println!(
            "   🎯裏 = G1 開催日の別場・非重賞（在庫偏在の可能性がある注目枠。-EV でも検証用に表示・張り推奨ではない）"
        );
    }

    let mut evaluations = Vec::with_capacity(due.len());
    for slot in due {
        let is_ura = is_g1_ura(slot.race.venue, slot.race_class, &day_classes);
        if let Some(evaluation) = evaluate_race(app, slot, is_ura, captured_at, ctx).await {
            evaluations.push(evaluation);
        }
    }
    evaluations
}

/// 1 レースを評価: フレッシュなオッズ再取得 → 確率推定 → 買い目/EV → ROI 判定。
///
/// 評価できたら通知の材料（[`RaceEvaluation`]）を返す（#584）。閾値判定はここではしない（`sweep` の doc 参照）。
/// オッズ不足等で評価に至らなかったレースは `None`。
async fn evaluate_race(
    app: &App,
    slot: &Slot,
    is_ura: bool,
    captured_at: &str,
    ctx: SweepCtx<'_>,
) -> Option<RaceEvaluation> {
    let SweepCtx {
        cli,
        notify_gate,
        blend_alpha,
        race_budget_overrides,
        pins,
        morning,
    } = ctx;
    let rid = &slot.race.race_id;
    let label = race_label(slot);
    // per-race 予算（#342）: override があればそのレースだけ増額、無ければ既定 --race-budget。
    // 軸・点数・相手は不変で金額のみ変わる（build_portfolio は budget を配分額にのみ使う）。
    let race_budget = resolve_race_budget(race_budget_overrides, rid.value(), cli.race_budget);

    // 1) 発走直前のフレッシュなオッズを再取得（新スナップショットを保存。read-through は使わない）。
    let odds = match app.odds.refresh_race_odds(rid).await {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("  {label}: オッズ未取得（未公開/失敗）、スキップ");
            return None;
        }
        // refresh_race_odds は現状スクレイプ失敗を Ok(None) に畳むため Err は来ないが、
        // 将来の戻り値変更（DB エラー伝播等）に備えて防御的に握っておく。
        Err(e) => {
            println!("  {label}: オッズ再取得エラー: {e}");
            return None;
        }
    };

    // 2) 確率を 2 視点で推定（#272 確率分離）。predict_race_views は factor 収集 1 回で blended（順位
    //    付け・市場ブレンド）と pure（EV 用・純モデル α=1.0）を返す。内部で find_race_odds（直前に
    //    persist した最新スナップショット）を再読込して blended のブレンドに使う。build_portfolio へ渡す
    //    odds と同一データだが読み出し経路は別。persist 失敗時（warn のみで継続）は旧スナップショットを
    //    見るため、その回だけ買い目側と確率側でオッズ集合が食い違いうる（次スイープで解消する一時的劣化）。
    //    track_condition は発走前レースでは None のため、監視は当日の馬場状態を反映しない。
    let views = match app
        .interactor
        .predict_race_views(rid, blend_alpha, slot.race.track_condition, true)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            println!("  {label}: 確率推定エラー: {e}");
            return None;
        }
    };

    // 近走データ皆無/過半欠損の警告（#552）。監視は ROI を出す decision-support なので、
    // 新馬戦・近走取得全滅レースは信頼性低を明示して回収率だけの候補入りを防ぐ。
    if let Some(warn) = format_recent_runs_warning(
        views.recent_runs_coverage.field_size,
        views.recent_runs_coverage.horses_with_runs,
    ) {
        // 密な監視ログでも埋もれないよう前に 1 行空けて浮かせる（他経路と同様に確率テーブル直前）。
        println!();
        println!("  {label}: {warn}");
    }

    // 過去データ視点（純モデルの順位＋根拠）。EV に依らず常に出す。エッジが無い窓でも「なぜこの順位か」を提示。
    println!("  ── {label} 過去データ視点（純モデル）");
    for line in format_probs(&views.pure) {
        println!("    {line}");
    }
    for line in format_explanations(&views.pure, &views.explanations) {
        println!("    {line}");
    }
    // 純モデル vs 市場implied（差で割安/割高の向きを読む）。
    let market_win: std::collections::HashMap<_, _> =
        odds.win.iter().map(|(num, o)| (*num, o.value())).collect();
    // 条件依存枠バイアスの複勝 lift（#343）。枠妙味フラグ（枠有利∧市場過小）判定に使う。
    let gate_lift: std::collections::HashMap<_, _> = views
        .explanations
        .iter()
        .filter_map(|e| e.gate_bias_lift.map(|l| (e.horse_num, l)))
        .collect();
    for line in format_probs_with_market(&views.pure, &market_win, &gate_lift) {
        println!("    {line}");
    }

    // 3) 買い目選定の固定（#601 軸ロック・#767 で朝の選定を優先）。rank_probs（市場ブレンド α=0.2）は
    //    市場 odds を含むため、固定しないとスイープごとに軸・相手・混戦判定が動く（実測 154R 中 60% しか
    //    終日一定でなかった）。記録◎（pad）は固定の元（朝の選定・初回スイープ）がまだ無いときだけ引く
    //    ——あればその軸に既に反映済みで（朝の選定は記録◎ を軸に強制して組む）、毎スイープ引くと
    //    Due レース数ぶんの N+1 になるため。
    let pin = pins.get(rid.value());
    let morning_sel = morning.get(rid.value());
    let recorded_axis = if reads_recorded_axis(pin.is_some(), morning_sel.is_some()) {
        app.interactor
            .repository
            .find_pad_prediction(cli.date, slot.race.venue, slot.race.race_num)
            .await
            .unwrap_or_else(|e| {
                // 記録◎が引けなくても監視は続ける（固定はライブ選定にフォールバックする）。
                println!("  {label}: 記録◎の取得に失敗（固定なしで継続）: {e}");
                None
            })
            .as_ref()
            .and_then(|pad| recorded_axis_of(Some(pad), &views.blended))
            .and_then(|n| HorseNum::try_from(n).ok())
    } else {
        None
    };
    let (pinned, source) = resolve_fixed_selection(recorded_axis, morning_sel, pin);
    print_pin_status(&label, &pinned, &source);

    // 4) 市場EV視点: 軸/相手は blended（固定があればそれを優先）、EV/的中は pure（循環断ち, #272）。
    let portfolio = compose_portfolio(&views, &odds, race_budget, &pinned);
    let Some(ev) = &portfolio.ev else {
        println!("  {label}: 買い目を組成できず（オッズ不足）、スキップ");
        return None;
    };
    print_pin_drift(&label, &pinned, &portfolio, &views.blended);

    // decision-support（#272）: 自動の張る/見送り判定はしない。純モデル EV/ROI は「市場に対しモデルが
    // 割安と見るか」の参考情報で、最終判断は人間のハンデ精査に委ねる。買う閾値（roi_gate）以上は 🔶、
    // 通知閾値（notify_gate）以上は 🔍 検証候補として表示に残す（#345・張り推奨ではない）。
    let roi_pct = ev.roi * 100.0;
    let hit_pct = ev.hit_prob * 100.0;
    let mark = mark_for(ev.roi, notify_gate, cli.roi_gate);
    // G1 裏（別場の非重賞）は ROI ゲートに関わらずタグを付けて注目枠として残す（#345）。
    let ura_tag = if is_ura { " 🎯裏" } else { "" };
    println!(
        "  {mark} {label}{ura_tag}: 参考ROI {roi_pct:.1}% / 的中 {hit_pct:.1}%（モデル単独視点・最終判断は手動精査）"
    );
    print_buy_targets(&portfolio);

    // 5) ライブ EV ビュー/アーカイブ（live_ev_snapshots）へ best-effort で永続化（#346 / ADR 0064）。
    //    ここに書いた選定が、次スイープ以降の固定（#601・`find_live_ev_pins_by_date`）の元になる。
    //    ここは decision-support のスナップショット記録であり、predict のセッション記録
    //    （predict_sessions / predict_bets）には触れない。保存失敗で監視ループは止めない。
    if let Some(axis) = portfolio.axis {
        // ◎の model 勝率[%]は blended（順位付け視点）の軸馬から採る。axis は build_portfolio が
        // blended の勝率最上位から選ぶため必ず blended に含まれる。unwrap_or(0.0) は到達しない前提の
        // 防御で、万一の不整合でも監視を止めず 0% として記録する（無言のクラッシュより望ましい）。
        let axis_prob = views
            .blended
            .iter()
            .find(|hp| hp.horse_num == axis)
            .map(|hp| hp.win_prob * 100.0)
            .unwrap_or(0.0);
        let axis_win_odds = odds.win.get(&axis).map(|o| o.value());
        let (axis_place_low, axis_place_high) = odds
            .place
            .get(&axis)
            .map(|b| (Some(b.low.value()), Some(b.high.value())))
            .unwrap_or((None, None));
        let post_time = slot.post_time.map(|t| t.format("%H:%M").to_string());
        // 荒れ度は純モデル勝率（odds 非依存のレース形状）から算出する（#344）。ROI とは別軸。
        let roughness =
            race_roughness(&views.pure.iter().map(|hp| hp.win_prob).collect::<Vec<_>>());

        let ctx = SnapshotContext {
            date: cli.date,
            race_id: rid.value(),
            venue: slot.race.venue.as_slug(),
            race_no: slot.race.race_num,
            post_time,
            captured_at,
            axis_prob,
            axis_win_odds,
            axis_place_odds_low: axis_place_low,
            axis_place_odds_high: axis_place_high,
            race_budget,
            roughness,
        };
        if let Some(record) = build_snapshot_record(&portfolio, &ctx)
            && let Err(e) = app.interactor.save_live_ev_snapshot(&record).await
        {
            println!("  {label}: ライブEVスナップショット保存に失敗（監視は継続）: {e}");
        }
    }

    // 6) 通知の材料を返す（#584）。ここでは閾値を見ない——発火判定と副作用は `WatchSweeper::sweep`
    //    側（`&mut self` を持つ唯一の場所）に集約する。
    Some(RaceEvaluation {
        race_id: rid.value().to_string(),
        label,
        race_name: slot.race_name.clone(),
        roi: ev.roi,
        axis: portfolio.axis.map(|a| a.value()),
    })
}

/// 張り候補の買い目を「そのまま買える形」で出力する（軸/相手＋各点）。整形は predict と共有する
/// `predict_format::format_portfolio` に委譲し（#452）、賭け計フッタだけ predict-watch 固有で付す。
/// predict-watch は 5 スペースインデント・0 円脚を落とす・未取得脚に EV を付けない設定。
///
/// 賭け計には**参考 ROI との基準のズレ**を注記する（#631）。上の行に出す参考 ROI は priced な脚
/// だけの値で、賭け計は全脚の合計。両者は行が離れているので、ズレの説明は賭け計側に付ける。
fn print_buy_targets(p: &Portfolio) {
    for line in format_portfolio(
        p,
        &PortfolioFormat {
            indent: "     ",
            skip_zero_stake: true,
            ev_on_unpriced: false,
        },
    ) {
        println!("{line}");
    }
    println!("     賭け計 ¥{}{}", p.total_stake, coverage_note(p));
}

/// 監視ループを駆動する [`Sweeper`]（#459）。共通の骨格（[`run_monitor_loop`]）へ predict-watch 固有の
/// slots ロード（post_time / race_class）・EV スイープ・windowed 設定・per-race override の初回チェックを注入する。
struct WatchSweeper<'a> {
    app: &'a App,
    cli: &'a Cli,
    window: Duration,
    notify_gate: f64,
    blend_alpha: Option<f64>,
    race_budget_overrides: HashMap<String, u64>,
    /// per-race override の pid が当日レースに 1 件も一致しないケースを 1 度だけ警告するためのフラグ。
    overrides_checked: bool,
    /// macOS 通知の発火閾値（run で 1 度だけ解決した参考ROI。#584）。表示ゲート `notify_gate` とは別物。
    /// 有効/無効は `cli.no_notify` が正で、ここには複製しない（二重管理を作らない）。
    notify_roi: f64,
    /// race_id → 前回**通知した**ときの参考ROI（#584 の重複抑制）。プロセス内メモリのみで永続化しない
    /// ——抑制は「1 プロセスの連投抑止」であって永続契約ではなく、永続化すると監視の再起動後に
    /// 本当に必要な初回通知を落とす（＝届かない）側に倒れる。
    last_notified_roi: HashMap<String, f64>,
    /// 配送失敗を報告済みか（#584）。**成功したら `delivery_report` が false へ戻す**ので、
    /// 一過性の失敗（画面ロック等）でその日いっぱい沈黙することはない。
    notify_failure_reported: bool,
}

impl WatchSweeper<'_> {
    /// `targets`（slots の添字）の朝の選定を計算・保存し、`morning` に足す（#767）。
    /// 1 レースの失敗では止めない。判断は `ensure_morning_selection` と `morning_targets` にあり、
    /// ここは呼び出しと集計だけ。
    async fn compute_morning(
        &self,
        slots: &[Slot],
        targets: &[usize],
        captured_at: &str,
        morning: &mut HashMap<String, MorningSelection>,
        tally: &mut MorningTally,
    ) {
        for &i in targets {
            let slot = &slots[i];
            // Due / NotYet は post_time を持つ（無ければ Unknown）
            let Some(post) = slot.post_time else { continue };
            let label = race_label(slot);
            let entry = window_entry_utc(self.cli.date, post, self.window);
            match self
                .app
                .interactor
                .ensure_morning_selection(&slot.race.race_id, self.cli.date, entry, captured_at)
                .await
            {
                Ok(MorningOutcome::Ready(sel)) => {
                    tally.saved += 1;
                    morning.insert(sel.race_id.clone(), sel);
                }
                Ok(MorningOutcome::NoSnapshot) => tally.no_snapshot += 1,
                Ok(MorningOutcome::TooLate { .. }) => tally.too_late += 1,
                Ok(MorningOutcome::NotComposable) => tally.not_composable += 1,
                Err(e) => {
                    tally.failed += 1;
                    println!("  {label}: 朝の選定の計算に失敗（初回スイープの固定で続ける）: {e}");
                }
            }
        }
    }

    /// 閾値を通過した評価を macOS 通知で人に届ける（#584）。発火判定は `pick_notifications`、
    /// 配送ループと状態遷移は `deliver_all`、配送そのものは `send_with_deadline` に委譲する。
    ///
    /// **このメソッドには判断を置かない**。`&App` / `&Cli` を抱えた構造体のメソッドはテストが
    /// 書けず、置いた判断は「消しても緑のまま」になる（2 巡目までの `record_notified` 呼び出しが
    /// まさにそうだった）。ここに残るのは有効/無効の分岐と印字だけ。
    async fn deliver_notifications(&mut self, evaluations: Vec<RaceEvaluation>) {
        // 配送手段が無いプラットフォームでは経路ごと落とす。試しても必ず失敗し、抑制状態も
        // 進まないので、同じレースが毎スイープ `🔔(未配送)` を積み上げる——#584 が減らそうと
        // した埋没を通知行側で再生産する。起動注記も同じ条件で「配送できません」を宣言している。
        if self.cli.no_notify || !PLATFORM_SUPPORTED {
            return;
        }
        let selected = pick_notifications(
            &self.last_notified_roi,
            evaluations,
            self.notify_roi,
            NOTIFY_ROI_RESEND_DELTA,
        );
        let reports = deliver_all(
            &mut self.last_notified_roi,
            &mut self.notify_failure_reported,
            selected,
            |message| async move { send_with_deadline(&message).await },
        )
        .await;
        for report in reports {
            println!("{}", report.line);
            if let Some(warning) = report.warning {
                println!("{warning}");
            }
        }
    }
}

impl Sweeper for WatchSweeper<'_> {
    type Slot = Slot;

    async fn load_slots(&self) -> anyhow::Result<Vec<Slot>> {
        load_slots(self.app, self.cli.date).await
    }

    fn post_time(slot: &Slot) -> Option<NaiveTime> {
        slot.post_time
    }

    fn has_result(slot: &Slot) -> bool {
        has_result(&slot.race)
    }

    fn window(&self) -> Option<Duration> {
        Some(self.window)
    }

    fn date(&self) -> chrono::NaiveDate {
        self.cli.date
    }

    fn once(&self) -> bool {
        self.cli.once
    }

    fn interval_minutes(&self) -> u64 {
        self.cli.interval
    }

    fn finish_noun(&self) -> &str {
        "監視"
    }

    async fn sweep(&mut self, slots: &[Slot], statuses: &[RaceStatus], now: NaiveTime) {
        // per-race override の pid が当日レースに 1 件も一致しなければ、タイプミス等の可能性を 1 度だけ
        // 警告する（黙って未適用のまま監視が進むのを防ぐ。#342）。slots が空の巡はまだ判定材料が無いので
        // 次巡に持ち越す（overrides_checked を立てない）。
        if !self.overrides_checked && !self.race_budget_overrides.is_empty() && !slots.is_empty() {
            let present: std::collections::HashSet<&str> =
                slots.iter().map(|s| s.race.race_id.value()).collect();
            let unmatched: Vec<&String> = self
                .race_budget_overrides
                .keys()
                .filter(|pid| !present.contains(pid.as_str()))
                .collect();
            if !unmatched.is_empty() {
                let list = unmatched
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "⚠ per-race 予算 override の race_id が当日（{}）のレースに一致しません（初回スイープの出馬表基準・未適用）: {list}。pid を確認してください（card 未取得なら fetch-card 後に再実行）。",
                    self.cli.date
                );
            }
            self.overrides_checked = true;
        }

        // 監視サイクル境界時刻（1 スイープ 1 値・UTC rfc3339 秒精度 Z 終端）。同一サイクルの全レースが
        // 同一 captured_at を共有し、live_ev_snapshots の「辞書順＝時刻順」「(race_id, captured_at) 冪等」
        // 契約を満たす（旧 refresh_ev.sh の `date -u +%Y-%m-%dT%H:%M:%SZ` と同表記）。
        let captured_at = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        // 買い目選定の固定（#601）: その日の初回スイープの選定を **1 スイープにつき 1 回**引く
        // （レースごとに引くと Due レース数ぶんの N+1 になる）。取得に失敗しても監視は止めず、
        // 固定なし＝従来どおりライブ選定へ縮退する（ただしその事実はログに残す）。
        let mut pins_loaded = true;
        let pins: HashMap<String, LiveEvPin> = match self
            .app
            .interactor
            .repository
            .find_live_ev_pins_by_date(self.cli.date)
            .await
        {
            Ok(v) => v.into_iter().map(|p| (p.race_id.clone(), p)).collect(),
            Err(e) => {
                println!(
                    "⚠ 買い目固定の読み出しに失敗しました（このスイープは固定なしで評価し、朝の選定も計算しません）: {e}"
                );
                pins_loaded = false;
                HashMap::new()
            }
        };
        // 朝の選定（#767）も 1 スイープにつき 1 回引く。失敗しても監視は止めず、初回スイープの固定へ縮退する。
        let mut morning: HashMap<String, MorningSelection> = match self
            .app
            .interactor
            .repository
            .find_morning_selections_by_date(self.cli.date)
            .await
        {
            Ok(v) => v.into_iter().map(|m| (m.race_id.clone(), m)).collect(),
            Err(e) => {
                println!(
                    "⚠ 朝の選定の読み出しに失敗しました（このスイープは初回スイープの固定で評価します）: {e}"
                );
                HashMap::new()
            }
        };
        let ids: Vec<&str> = slots.iter().map(|s| s.race.race_id.value()).collect();
        let (due_targets, later_targets) = morning_targets(&ids, statuses, pins_loaded, |id| {
            pins.contains_key(id) || morning.contains_key(id)
        });
        let mut tally = MorningTally::default();
        // 窓に入ったレースは評価の前に計算する（このスイープの評価から朝の選定で固定する）
        self.compute_morning(slots, &due_targets, &captured_at, &mut morning, &mut tally)
            .await;
        let ctx = SweepCtx {
            cli: self.cli,
            notify_gate: self.notify_gate,
            blend_alpha: self.blend_alpha,
            race_budget_overrides: &self.race_budget_overrides,
            pins: &pins,
            morning: &morning,
        };
        let evaluations = sweep(self.app, slots, statuses, now, &captured_at, ctx).await;
        // 判定をログに埋もれさせず人に届ける（#584）。評価の直後に出し、窓の前のレースの朝の計算
        // （起動直後は全レース分で数十秒かかりうる）で通知を遅らせない。
        self.deliver_notifications(evaluations).await;
        // 窓の前のレースは評価と通知の後に計算する（初回スイープの評価と通知を遅らせない）
        self.compute_morning(
            slots,
            &later_targets,
            &captured_at,
            &mut morning,
            &mut tally,
        )
        .await;
        if let Some(line) = tally.line() {
            println!("{line}");
        }
    }
}

/// 監視ループ。発走前のレースが残っている間スキャンを繰り返し、全レース発走で自動終了する。
pub async fn run(app: &App, cli: &Cli) -> anyhow::Result<()> {
    // 連続再取得（busy loop）で JRA を叩き続けないよう、間隔・窓は 1 分以上を要求する。
    if cli.interval == 0 {
        anyhow::bail!(
            "--interval は 1 分以上を指定してください（0 は JRA への連続再取得になり礼節に反します）。"
        );
    }
    if cli.window == 0 {
        anyhow::bail!("--window は 1 分以上を指定してください。");
    }
    // α は市場とモデルのブレンド重み（0=市場のみ, 1=モデルのみ）。範囲外は無意味なので弾く。
    if let Some(a) = cli.blend_alpha
        && !(0.0..=1.0).contains(&a)
    {
        anyhow::bail!(
            "--blend-alpha は 0.0〜1.0 で指定してください（市場とモデルのブレンド重み）。"
        );
    }
    // 買う閾値そのものの健全性を先に見る（#584 レビュー）。非有限だと `resolve_notify_roi` の
    // 追従経路（未指定＝roi_gate）がそのまま NaN を受け取り、表示・通知の判定が丸ごと壊れる。
    if !cli.roi_gate.is_finite() || cli.roi_gate < 0.0 {
        anyhow::bail!(
            "--roi-gate（{}）は 0 以上の有限値で指定してください（NaN/∞ は表示マークと通知の判定を無言で壊します）。",
            cli.roi_gate
        );
    }
    // 表示（検証候補 🔍）閾値を解決する。未指定は min(roi_gate, 0.7)、明示指定の逆転（buy 超）は弾く（#345）。
    let notify_gate = resolve_notify_gate(cli.notify_gate, cli.roi_gate)?;
    // macOS 通知の発火閾値を解決する（#584）。上の表示閾値とは別物で、未指定は roi_gate と同値。
    let notify_roi = resolve_notify_roi(cli.notify_roi, cli.roi_gate)?;

    // α 未指定なら本番既定（market α=0.2）を使う。predict と同一値で判定を揃える。
    let blend_alpha = cli.blend_alpha.or(RECOMMENDED_MARKET_BLEND_ALPHA);
    let window = Duration::minutes(cli.window as i64);

    // per-race 予算 override（#342）。起動時に 1 度だけパース・検証し、適用一覧を提示する。
    // 当日レースに無い race_id（pid タイプミス等）は初回スイープの slots ロード後に 1 度だけ警告する。
    let race_budget_overrides = parse_race_budget_overrides(&cli.race_budget_overrides)?;
    if !race_budget_overrides.is_empty() {
        let mut applied: Vec<(&String, &u64)> = race_budget_overrides.iter().collect();
        applied.sort_by(|a, b| a.0.cmp(b.0));
        let list = applied
            .iter()
            .map(|(pid, yen)| format!("{pid}=¥{yen}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "── per-race 予算 override: {} 件適用（{list}）。指定レースは軸・点数・相手を変えず金額のみ増減。",
            race_budget_overrides.len()
        );
    }

    // 発走状態は実行マシンの現在時刻と post_time の「時刻」だけで判定するため、当日以外の date や
    // JST 以外の TZ では判定が無意味になる。誤用に早期に気づけるよう起動時に警告する（#459 で共通化）。
    warn_if_not_today_jst_now(cli.date, "発走状態");

    // 参考ROI の読み方（#602 / ADR 0079）。**起動時 1 回だけ**出す——スイープごとに出すと
    // 1 開催日で 80 回を超え、肝心の判定行が埋もれる（#584 が問題にした「ログに埋もれる」の再生産）。
    print_gate_caveat(cli.roi_gate, notify_gate);

    // 通知設定も起動時 1 回だけ宣言する（#584）。**これが無いと「鳴らない＝妙味なし」と
    // 「そもそも既定閾値では鳴らない」が区別できず**、監視の失敗が沈黙として現れる
    // （knowledge/monitor-loop-sleep-resilience.md）状態を通知側にも作ってしまう。
    for line in notify_status_lines(&NotifySettings {
        enabled: !cli.no_notify,
        notify_roi,
        explicit: cli.notify_roi.is_some(),
        roi_gate: cli.roi_gate,
        notify_gate,
        delta: NOTIFY_ROI_RESEND_DELTA,
        platform_supported: PLATFORM_SUPPORTED,
    }) {
        println!("{line}");
    }

    let mut sweeper = WatchSweeper {
        app,
        cli,
        window,
        notify_gate,
        blend_alpha,
        race_budget_overrides,
        overrides_checked: false,
        notify_roi,
        last_notified_roi: HashMap::new(),
        notify_failure_reported: false,
    };
    run_monitor_loop(&mut sweeper).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // 固定の優先順（朝の選定 → 初回スイープ → 記録◎ の軸だけ → なし）は use-case の `resolve_fixed_selection` の
    // テストが持つ（盤と共有するため）。ここは predict-watch に残る判断だけを見る。

    #[test]
    fn window_entry_is_post_time_minus_window_in_utc() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
        let post = NaiveTime::from_hms_opt(15, 40, 0).unwrap();
        // JST 15:40 − 40 分 = JST 15:00 = UTC 06:00
        assert_eq!(
            window_entry_utc(d, post, Duration::minutes(40)).to_rfc3339(),
            "2026-10-10T06:00:00+00:00"
        );
        // 午前 0 時台の発走は前日の UTC になる
        let early = NaiveTime::from_hms_opt(0, 30, 0).unwrap();
        assert_eq!(
            window_entry_utc(d, early, Duration::minutes(40)).to_rfc3339(),
            "2026-10-09T14:50:00+00:00"
        );
    }

    #[test]
    fn morning_targets_skip_fixed_and_finished_races() {
        let ids = ["r1", "r2", "r3", "r4", "r5", "r6"];
        let statuses = [
            RaceStatus::Due,     // r1: 窓内・未固定 → 評価の前に計算
            RaceStatus::Due,     // r2: 窓内だが初回スイープで固定済み → 計算しない
            RaceStatus::NotYet,  // r3: 窓の前・未固定 → 評価の後に計算
            RaceStatus::NotYet,  // r4: 窓の前だが朝の選定がある → 計算しない
            RaceStatus::Started, // r5: 発走済み
            RaceStatus::Unknown, // r6: 発走時刻不明
        ];
        let (due, later) = morning_targets(&ids, &statuses, true, |id| id == "r2" || id == "r4");
        assert_eq!(due, vec![0]);
        assert_eq!(later, vec![2]);
        // 初回スイープの固定を読めなかったスイープでは何も計算しない
        let (due, later) = morning_targets(&ids, &statuses, false, |_| false);
        assert!(due.is_empty() && later.is_empty());
    }

    #[test]
    fn recorded_axis_is_read_only_without_any_fix() {
        assert!(reads_recorded_axis(false, false));
        assert!(
            !reads_recorded_axis(true, false),
            "初回スイープの軸に反映済み"
        );
        assert!(
            !reads_recorded_axis(false, true),
            "朝の選定の後の記録◎ は使わない"
        );
        assert!(!reads_recorded_axis(true, true));
    }

    #[test]
    fn morning_tally_is_silent_when_nothing_was_attempted() {
        assert_eq!(MorningTally::default().line(), None);
        let t = MorningTally {
            saved: 2,
            too_late: 1,
            ..MorningTally::default()
        };
        let line = t.line().unwrap();
        assert!(line.contains("確定 2"), "{line}");
        assert!(
            line.contains("窓に入った後の単勝しか無い 1（T-40 で固定）"),
            "{line}"
        );
    }

    #[test]
    fn jst_hhmm_converts_utc_and_passes_through_garbage() {
        assert_eq!(jst_hhmm("2026-08-09T00:15:03Z"), "09:15");
        assert_eq!(jst_hhmm("2026-08-09T09:27:37+00:00"), "18:27");
        // 解釈できない値でも監視を止めず、そのまま出して人が気づけるようにする。
        assert_eq!(jst_hhmm("not-a-time"), "not-a-time");
    }

    /// 起動注記は「到達 0 件」と ADR 0076 を必ず含む（実測に紐づく文言なので落とさない）。
    #[test]
    fn gate_caveat_states_unreachable_with_evidence() {
        let lines = gate_caveat_lines(1.0, 0.7);
        let joined = lines.join("\n");
        assert!(joined.contains("到達 0 件"), "{joined}");
        assert!(joined.contains("ADR 0076"), "{joined}");
        assert!(joined.contains("≥100%"), "roi_gate を反映する: {joined}");
        assert!(joined.contains("≥70%"), "notify_gate を反映する: {joined}");
        // 張る判断の拠り所（手動精査＋執行の規律）まで書いて初めて「当てにしない」が伝わる。
        assert!(joined.contains("軸ロック"), "{joined}");
    }

    #[test]
    fn gate_caveat_does_not_claim_zero_hits_below_the_measured_gate() {
        // #584: ADR 0076 が「到達 0 件」と実測したのは ≥100%。--roi-gate を下げた探索運用でも
        // 同じ断言を出すと、測っていない閾値について到達不能を主張することになる
        // （一次資料は 2026-08-09 に ≥70% の通過を 7 回観測）。通知側の
        // `notify_status_does_not_cite_adr0076_below_the_measured_gate` と対になる検査。
        let lines = gate_caveat_lines(0.7, 0.7);
        assert!(lines[0].contains("ADR 0076"));
        assert!(
            !lines[0].contains("実測で到達 0 件"),
            "測っていない閾値について到達 0 件と断言している: {}",
            lines[0]
        );
        assert!(
            lines[0].contains("≥70%") && lines[0].contains("≥100%"),
            "{}",
            lines[0]
        );
    }

    /// `notify_gate == roi_gate` のとき 🔍 帯は構造的に空なので、その行は出さない
    /// （出ないマークを案内しない＝スイープヘッダの既存規約と揃える）。
    #[test]
    fn gate_caveat_omits_notify_line_when_band_is_empty() {
        let joined = gate_caveat_lines(1.0, 1.0).join("\n");
        assert!(!joined.contains("🔍"), "{joined}");
        assert!(joined.contains("到達 0 件"), "{joined}");
    }

    #[test]
    fn is_g1_ura_detects_other_venue_nongraded_on_g1_day() {
        // 東京で G1、函館は非重賞（未勝利）→ 函館の当該は G1 裏。
        let day = [
            (Venue::Tokyo, Some(RaceClass::G1)),
            (Venue::Hakodate, Some(RaceClass::Maiden)),
        ];
        assert!(is_g1_ura(Venue::Hakodate, Some(RaceClass::Maiden), &day));
    }

    #[test]
    fn is_g1_ura_false_for_same_venue_as_g1() {
        // G1 開催場（東京）の平場は「別場」でないため裏に含めない。
        let day = [
            (Venue::Tokyo, Some(RaceClass::G1)),
            (Venue::Tokyo, Some(RaceClass::Open)),
        ];
        assert!(!is_g1_ura(Venue::Tokyo, Some(RaceClass::Open), &day));
    }

    #[test]
    fn is_g1_ura_false_for_graded_race() {
        // 別場でも重賞（G3）は非重賞でないため裏でない。
        let day = [
            (Venue::Tokyo, Some(RaceClass::G1)),
            (Venue::Hakodate, Some(RaceClass::G3)),
        ];
        assert!(!is_g1_ura(Venue::Hakodate, Some(RaceClass::G3), &day));
    }

    #[test]
    fn is_g1_ura_false_when_no_g1_today() {
        // 当日 G1 が無ければ裏は成立しない。
        let day = [
            (Venue::Tokyo, Some(RaceClass::G3)),
            (Venue::Hakodate, Some(RaceClass::Maiden)),
        ];
        assert!(!is_g1_ura(Venue::Hakodate, Some(RaceClass::Maiden), &day));
    }

    #[test]
    fn is_g1_ura_false_when_class_unknown() {
        // クラス不明は非重賞と断定できないため裏にしない。
        let day = [(Venue::Tokyo, Some(RaceClass::G1)), (Venue::Hakodate, None)];
        assert!(!is_g1_ura(Venue::Hakodate, None, &day));
    }

    #[test]
    fn parse_race_budget_overrides_parses_multiple_entries() {
        let entries = vec![
            "2026-3-hakodate-2-6R=7000".to_string(),
            "2026-3-hakodate-2-7R=5500".to_string(),
        ];
        let map = parse_race_budget_overrides(&entries).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("2026-3-hakodate-2-6R"), Some(&7000));
        assert_eq!(map.get("2026-3-hakodate-2-7R"), Some(&5500));
    }

    #[test]
    fn parse_race_budget_overrides_empty_is_empty_map() {
        assert!(parse_race_budget_overrides(&[]).unwrap().is_empty());
    }

    #[test]
    fn parse_race_budget_overrides_trims_whitespace() {
        let entries = vec![" 2026-3-tokyo-5-11R = 8000 ".to_string()];
        let map = parse_race_budget_overrides(&entries).unwrap();
        assert_eq!(map.get("2026-3-tokyo-5-11R"), Some(&8000));
    }

    #[test]
    fn parse_race_budget_overrides_rejects_missing_eq() {
        assert!(parse_race_budget_overrides(&["2026-3-tokyo-5-11R".to_string()]).is_err());
    }

    #[test]
    fn parse_race_budget_overrides_rejects_under_100_and_non_numeric() {
        // 0 円・100 円未満は空伝票になるため弾く（増額用途として無意味）。
        assert!(parse_race_budget_overrides(&["2026-3-tokyo-5-11R=0".to_string()]).is_err());
        assert!(parse_race_budget_overrides(&["2026-3-tokyo-5-11R=50".to_string()]).is_err());
        assert!(parse_race_budget_overrides(&["2026-3-tokyo-5-11R=abc".to_string()]).is_err());
        // 負値は u64 パース失敗で弾かれる。
        assert!(parse_race_budget_overrides(&["2026-3-tokyo-5-11R=-100".to_string()]).is_err());
        // 100 円ちょうどは許容（境界）。
        assert_eq!(
            parse_race_budget_overrides(&["2026-3-tokyo-5-11R=100".to_string()])
                .unwrap()
                .get("2026-3-tokyo-5-11R"),
            Some(&100)
        );
    }

    #[test]
    fn parse_race_budget_overrides_rejects_invalid_race_id() {
        // RaceId は英数・`-` のみ許可（`_` は不可）。不正文字を含む pid を弾く。
        assert!(parse_race_budget_overrides(&["bad_id=7000".to_string()]).is_err());
        assert!(parse_race_budget_overrides(&["=7000".to_string()]).is_err());
    }

    #[test]
    fn parse_race_budget_overrides_rejects_duplicate_race_id() {
        let entries = vec![
            "2026-3-tokyo-5-11R=7000".to_string(),
            "2026-3-tokyo-5-11R=8000".to_string(),
        ];
        assert!(parse_race_budget_overrides(&entries).is_err());
    }

    #[test]
    fn resolve_race_budget_uses_override_then_default() {
        let mut overrides = HashMap::new();
        overrides.insert("2026-3-tokyo-5-6R".to_string(), 7000);
        // override 有: per-race 値。
        assert_eq!(
            resolve_race_budget(&overrides, "2026-3-tokyo-5-6R", 5000),
            7000
        );
        // override 無: 既定。
        assert_eq!(
            resolve_race_budget(&overrides, "2026-3-tokyo-5-7R", 5000),
            5000
        );
        // 空 override: 常に既定。
        assert_eq!(
            resolve_race_budget(&HashMap::new(), "2026-3-tokyo-5-6R", 5000),
            5000
        );
    }

    #[test]
    fn resolve_notify_gate_defaults_and_clamps() {
        // 未指定 → min(roi_gate, 0.7)。
        assert_eq!(resolve_notify_gate(None, 1.0).unwrap(), 0.7);
        // roi_gate を 0.7 未満へ下げた探索運用は起動を壊さずクランプ。
        assert_eq!(resolve_notify_gate(None, 0.6).unwrap(), 0.6);
        // 明示指定はそのまま。
        assert_eq!(resolve_notify_gate(Some(0.8), 1.0).unwrap(), 0.8);
        // 明示指定が buy 閾値ちょうどは許容（境界）。
        assert_eq!(resolve_notify_gate(Some(1.0), 1.0).unwrap(), 1.0);
        // 明示指定が buy 閾値超は誤用として弾く。
        assert!(resolve_notify_gate(Some(0.9), 0.8).is_err());
        // 非有限・負値は弾く（NaN は比較が常に false で無言劣化するため）。
        assert!(resolve_notify_gate(Some(f64::NAN), 1.0).is_err());
        assert!(resolve_notify_gate(Some(f64::INFINITY), 1.0).is_err());
        assert!(resolve_notify_gate(Some(-0.1), 1.0).is_err());
    }

    #[test]
    fn mark_for_tiers_by_gates() {
        // buy_gate=1.0 / notify_gate=0.7 の既定帯。
        assert_eq!(mark_for(1.20, 0.7, 1.0), "🔶", "買う閾値以上は買い妙味");
        assert_eq!(mark_for(1.00, 0.7, 1.0), "🔶", "買う閾値ちょうども 🔶");
        assert_eq!(mark_for(0.85, 0.7, 1.0), "🔍", "検証候補帯");
        assert_eq!(mark_for(0.70, 0.7, 1.0), "🔍", "通知閾値ちょうども 🔍");
        assert_eq!(mark_for(0.69, 0.7, 1.0), "・", "通知閾値未満は低シグナル");
    }
}
