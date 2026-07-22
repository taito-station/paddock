use std::collections::HashMap;

use chrono::{Duration, NaiveTime, Utc};
use monitor_loop::{RaceStatus, Sweeper, has_result, run_monitor_loop, warn_if_not_today_jst_now};
use paddock_domain::{
    BetMethod, Portfolio, PortfolioConfig, RECOMMENDED_MARKET_BLEND_ALPHA, Race, RaceClass, RaceId,
    Venue, build_portfolio, race_roughness,
};
use paddock_use_case::PredictionViews;
use predict_format::{format_explanations, format_probs, format_probs_with_market};

use crate::cli::Cli;
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
}

/// 発走時刻付きのレース 1 件（races_by_date の Race ＋ race_card の post_time / race_class）。
struct Slot {
    race: Race,
    post_time: Option<NaiveTime>,
    /// レースクラス（#345・race_card 由来）。G1 裏レース検出に使う。未取得・判定不能は None。
    race_class: Option<RaceClass>,
}

/// 指定日の全レースを取得し、各レースの post_time / race_class（race_card 由来）を引き当てる。
///
/// #459 で per-race `race_card`（N+1）を日付一括クエリ 2 本（post_time / race_class）に置き換えた。
/// races_by_date の各レースに対し、一括マップから引く（マップに無い＝未保存 NULL は None＝旧 per-card
/// 経路で `card.post_time`/`card.race_class` が None だったのと同一集合になる）。
async fn load_slots(app: &App, date: chrono::NaiveDate) -> anyhow::Result<Vec<Slot>> {
    let races = app.interactor.races_by_date(date).await?;
    let post_times = app.interactor.post_times_by_date(date).await?;
    let race_classes = app.interactor.race_classes_by_date(date).await?;
    let mut slots = Vec::with_capacity(races.len());
    for race in races {
        let post_time = post_times.get(&race.race_id).copied();
        let race_class = race_classes.get(&race.race_id).copied();
        slots.push(Slot {
            race,
            post_time,
            race_class,
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
async fn sweep(
    app: &App,
    slots: &[Slot],
    statuses: &[RaceStatus],
    now: NaiveTime,
    captured_at: &str,
    ctx: SweepCtx<'_>,
) {
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
        format!(" ・ 🔍検証候補≥{:.0}%", notify_gate * 100.0)
    } else {
        String::new()
    };
    println!(
        "── {} スイープ: 対象 {} レース（窓 {}分 / 🔶買い妙味≥{:.0}%{}・判定は手動精査）",
        now.format("%H:%M"),
        due.len(),
        cli.window,
        cli.roi_gate * 100.0,
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

    for slot in due {
        let is_ura = is_g1_ura(slot.race.venue, slot.race_class, &day_classes);
        evaluate_race(app, slot, is_ura, captured_at, ctx).await;
    }
}

/// 1 レースを評価: フレッシュなオッズ再取得 → 確率推定 → 買い目/EV → ROI 判定。
async fn evaluate_race(app: &App, slot: &Slot, is_ura: bool, captured_at: &str, ctx: SweepCtx<'_>) {
    let SweepCtx {
        cli,
        notify_gate,
        blend_alpha,
        race_budget_overrides,
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
            return;
        }
        // refresh_race_odds は現状スクレイプ失敗を Ok(None) に畳むため Err は来ないが、
        // 将来の戻り値変更（DB エラー伝播等）に備えて防御的に握っておく。
        Err(e) => {
            println!("  {label}: オッズ再取得エラー: {e}");
            return;
        }
    };

    // 2) 確率を 2 視点で推定（#272 確率分離）。predict_race_views は factor 収集 1 回で blended（順位
    //    付け・市場ブレンド）と pure（EV 用・純モデル α=1.0）を返す。内部で find_race_odds（直前に
    //    persist した最新スナップショット）を再読込して blended のブレンドに使う。build_portfolio へ渡す
    //    odds と同一データだが読み出し経路は別。persist 失敗時（warn のみで継続）は旧スナップショットを
    //    見るため、その回だけ買い目側と確率側でオッズ集合が食い違いうる（次スイープで解消する一時的劣化）。
    //    track_condition は発走前レースでは None のため、監視は当日の馬場状態を反映しない。
    let PredictionViews {
        blended,
        pure,
        explanations,
    } = match app
        .interactor
        .predict_race_views(rid, blend_alpha, slot.race.track_condition, true)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            println!("  {label}: 確率推定エラー: {e}");
            return;
        }
    };

    // 過去データ視点（純モデルの順位＋根拠）。EV に依らず常に出す。エッジが無い窓でも「なぜこの順位か」を提示。
    println!("  ── {label} 過去データ視点（純モデル）");
    for line in format_probs(&pure) {
        println!("    {line}");
    }
    for line in format_explanations(&pure, &explanations) {
        println!("    {line}");
    }
    // 純モデル vs 市場implied（差で割安/割高の向きを読む）。
    let market_win: std::collections::HashMap<_, _> =
        odds.win.iter().map(|(num, o)| (*num, o.value())).collect();
    // 条件依存枠バイアスの複勝 lift（#343）。枠妙味フラグ（枠有利∧市場過小）判定に使う。
    let gate_lift: std::collections::HashMap<_, _> = explanations
        .iter()
        .filter_map(|e| e.gate_bias_lift.map(|l| (e.horse_num, l)))
        .collect();
    for line in format_probs_with_market(&pure, &market_win, &gate_lift) {
        println!("    {line}");
    }

    // 3) 市場EV視点: 軸/相手は blended、EV/的中は pure（循環断ち, #272）。
    let portfolio = build_portfolio(
        &blended,
        &pure,
        &odds,
        race_budget,
        &PortfolioConfig::default(),
    );
    let Some(ev) = &portfolio.ev else {
        println!("  {label}: 買い目を組成できず（オッズ不足）、スキップ");
        return;
    };

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

    // 4) ライブ EV ビュー/アーカイブ（live_ev_snapshots）へ best-effort で永続化（#346 / ADR 0064）。
    //    ここは decision-support のスナップショット記録であり、predict のセッション記録
    //    （predict_sessions / predict_bets）には触れない。保存失敗で監視ループは止めない。
    if let Some(axis) = portfolio.axis {
        // ◎の model 勝率[%]は blended（順位付け視点）の軸馬から採る。axis は build_portfolio が
        // blended の勝率最上位から選ぶため必ず blended に含まれる。unwrap_or(0.0) は到達しない前提の
        // 防御で、万一の不整合でも監視を止めず 0% として記録する（無言のクラッシュより望ましい）。
        let axis_prob = blended
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
        let roughness = race_roughness(&pure.iter().map(|hp| hp.win_prob).collect::<Vec<_>>());

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
}

/// 張り候補の買い目を「そのまま買える形」で出力する（軸/相手＋各点）。
fn print_buy_targets(p: &Portfolio) {
    if let Some(axis) = p.axis {
        let rel = p
            .partners
            .iter()
            .map(|h| h.value().to_string())
            .collect::<Vec<_>>()
            .join(",");
        println!("     軸 {} → 相手 {}", axis.value(), rel);
    }
    if p.bets.iter().any(|b| b.method == BetMethod::Box) {
        println!("     混戦: 印馬3連複ボックス（軸なし）を併用");
    }
    for bet in &p.bets {
        if bet.stake == 0 {
            continue;
        }
        // 方式（ながし/ボックス）を明示。box は軸なしの印馬総当たりで「軸流し」枠と区別する（表記規約）。
        let method = match bet.method {
            BetMethod::Nagashi => "ながし",
            BetMethod::Box => "ボックス",
        };
        match bet.odds {
            Some(o) => println!(
                "     [{}] {} ¥{} オッズ{:.1} 的中{:.1}% EV={:.2}",
                method,
                bet.combination.label_ja(),
                bet.stake,
                o,
                bet.hit_prob * 100.0,
                bet.ev,
            ),
            None => println!(
                "     [{}] {} ¥{} オッズ未取得 的中{:.1}%",
                method,
                bet.combination.label_ja(),
                bet.stake,
                bet.hit_prob * 100.0,
            ),
        }
    }
    println!("     賭け計 ¥{}", p.total_stake);
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
        let ctx = SweepCtx {
            cli: self.cli,
            notify_gate: self.notify_gate,
            blend_alpha: self.blend_alpha,
            race_budget_overrides: &self.race_budget_overrides,
        };
        sweep(self.app, slots, statuses, now, &captured_at, ctx).await;
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
    // 通知（検証候補）閾値を解決する。未指定は min(roi_gate, 0.7)、明示指定の逆転（buy 超）は弾く（#345）。
    let notify_gate = resolve_notify_gate(cli.notify_gate, cli.roi_gate)?;

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

    let mut sweeper = WatchSweeper {
        app,
        cli,
        window,
        notify_gate,
        blend_alpha,
        race_budget_overrides,
        overrides_checked: false,
    };
    run_monitor_loop(&mut sweeper).await
}

#[cfg(test)]
mod tests {
    use super::*;

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
