use std::time::Duration as StdDuration;

use chrono::{Duration, Local, NaiveTime, Offset, Utc};
use paddock_domain::{
    BetMethod, Portfolio, PortfolioConfig, RECOMMENDED_MARKET_BLEND_ALPHA, Race, RaceClass, Venue,
    build_portfolio,
};
use paddock_use_case::PredictionViews;
use predict_format::{format_explanations, format_probs, format_probs_with_market};

use crate::cli::Cli;
use crate::setup::App;
use crate::snapshot::{SnapshotContext, build_snapshot_record};

/// `now` 時点でのレースの発走状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceStatus {
    /// 発走前で先読み窓内 → オッズ再取得＆EV 再計算の対象。
    Due,
    /// 発走前だが窓より先 → まだ対象外（次スイープ以降に Due 化）。
    NotYet,
    /// 発走済み（結果取込済み or 発走時刻超過）→ 対象外。
    Started,
    /// 発走時刻不明（post_time 無し）→ 判定不能、対象外。
    Unknown,
}

/// `now` 時点でのレース発走状態を判定する純関数（単体テスト対象）。
///
/// - post_time 無し → `Unknown`
/// - 結果取込済み（`has_result`）または発走時刻超過（`now > post`）→ `Started`
/// - 発走前で残り時間が `window` 以内 → `Due`、それより先 → `NotYet`
pub fn classify(
    now: NaiveTime,
    post_time: Option<NaiveTime>,
    has_result: bool,
    window: Duration,
) -> RaceStatus {
    let Some(post) = post_time else {
        return RaceStatus::Unknown;
    };
    if has_result || now > post {
        return RaceStatus::Started;
    }
    // ここで now <= post。発走まで (post - now)。窓内なら Due。
    if post - now <= window {
        RaceStatus::Due
    } else {
        RaceStatus::NotYet
    }
}

/// 通知（検証候補）閾値の既定値。買う閾値を下回る帯を 🔍 として残す（#345）。
const DEFAULT_NOTIFY_GATE: f64 = 0.7;

/// 通知（検証候補）閾値を解決する純関数（#345・単体テスト対象）。
///
/// - 明示指定なし → `min(roi_gate, DEFAULT_NOTIFY_GATE)`。roi_gate を既定 0.7 未満へ下げた
///   探索運用でも「notify_gate > roi_gate」で起動が壊れないようクランプする。
/// - 明示指定あり → その値。ただし buy 閾値 `roi_gate` 超は 🔍 帯が生じない誤用として弾く。
pub fn resolve_notify_gate(explicit: Option<f64>, roi_gate: f64) -> anyhow::Result<f64> {
    match explicit {
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
/// `notify_gate <= buy_gate` を前提とする（run で検証済み）。表示のみで、買う/見送りの判断や
/// DB snapshot の verdict には影響しない（decision-support・軸ロック）。
pub fn mark_for(roi: f64, notify_gate: f64, buy_gate: f64) -> &'static str {
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

/// 監視を継続すべきか（発走前のレースが残っているか）を判定する純関数（単体テスト対象）。
/// Due か NotYet が 1 つでもあれば継続、無ければ終了。
pub fn should_continue(statuses: &[RaceStatus]) -> bool {
    statuses
        .iter()
        .any(|s| matches!(s, RaceStatus::Due | RaceStatus::NotYet))
}

/// 結果取込済み（＝確実に発走済み）か。`races_by_date` は発走前レースを race_cards 由来で
/// track_condition=NULL・results 空として返す（fact-check 済みの不変条件）ため、この経路では
/// track_condition/results は「成績取込が済んだ＝確実に過去のレース」を指す早期シグナル。
/// 成績取込前でも発走済みになる通常遷移（発走直後）は classify の `now > post` 側が捕捉する。
fn has_result(race: &Race) -> bool {
    race.track_condition.is_some() || !race.results.is_empty()
}

/// 1 スイープ全体で共有する評価コンテキスト（#345）。sweep / evaluate_race の引数肥大を避ける。
/// notify_gate は run で 1 度だけ解決した値、blend_alpha は解決済みの市場ブレンド重み。
#[derive(Clone, Copy)]
struct SweepCtx<'a> {
    cli: &'a Cli,
    notify_gate: f64,
    blend_alpha: Option<f64>,
}

/// 発走時刻付きのレース 1 件（races_by_date の Race ＋ race_card の post_time / race_class）。
struct Slot {
    race: Race,
    post_time: Option<NaiveTime>,
    /// レースクラス（#345・race_card 由来）。G1 裏レース検出に使う。未取得・判定不能は None。
    race_class: Option<RaceClass>,
}

/// 指定日の全レースを取得し、各レースの post_time / race_class（race_card 由来）を引き当てる。
async fn load_slots(app: &App, date: chrono::NaiveDate) -> anyhow::Result<Vec<Slot>> {
    let races = app.interactor.races_by_date(date).await?;
    let mut slots = Vec::with_capacity(races.len());
    for race in races {
        // race_card は 1 回だけ引き、post_time と race_class の両方を取り出す。
        let card = app.interactor.race_card(&race.race_id).await?;
        let post_time = card.as_ref().and_then(|c| c.post_time);
        let race_class = card.and_then(|c| c.race_class);
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

    println!(
        "── {} スイープ: 対象 {} レース（窓 {}分 / 🔶買い妙味≥{:.0}% ・ 🔍検証候補≥{:.0}%・判定は手動精査）",
        now.format("%H:%M"),
        due.len(),
        cli.window,
        cli.roi_gate * 100.0,
        notify_gate * 100.0,
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
    } = ctx;
    let rid = &slot.race.race_id;
    let label = race_label(slot);

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
        cli.race_budget,
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
            race_budget: cli.race_budget,
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

    // 発走状態は実行マシンの現在時刻と post_time の「時刻」だけで判定するため、(1) 当日以外の date、
    // (2) JST 以外の TZ では判定が無意味になる。誤用に早期に気づけるよう起動時に警告する。
    const JST_OFFSET_SECS: i32 = 9 * 3600;
    let now_local = Local::now();
    let today = now_local.date_naive();
    if cli.date != today {
        println!(
            "⚠ --date {} は本日（{today}）と異なります。発走状態は現在時刻と post_time の時刻のみで \
             判定するため、当日以外の指定では Due/Started 判定が正しく機能しません。",
            cli.date,
        );
    }
    let tz_offset = now_local.offset().fix().local_minus_utc();
    if tz_offset != JST_OFFSET_SECS {
        // 半端な TZ（例 +05:30）も正しく出せるよう ±HH:MM 表記にする。
        let sign = if tz_offset < 0 { '-' } else { '+' };
        let abs = tz_offset.abs();
        println!(
            "⚠ 実行マシンのタイムゾーンが JST(+09:00) ではありません（現在 UTC{sign}{:02}:{:02}）。\
             post_time は JST 起算のため、発走状態判定がオフセットぶんずれます。JST マシンで実行してください。",
            abs / 3600,
            (abs % 3600) / 60,
        );
    }

    loop {
        // 継続監視中の一時的 DB エラーでプロセスを落とすと「唯一エッジのある局面」を取りこぼす。
        // evaluate_race と同じく握って次スイープへ続行する（--once 時のみ伝播して非ゼロ終了）。
        let slots = match load_slots(app, cli.date).await {
            Ok(s) => s,
            Err(e) if cli.once => return Err(e),
            Err(e) => {
                println!("⚠ レース一覧の取得に失敗（次スイープで再試行）: {e}");
                tokio::time::sleep(StdDuration::from_secs(cli.interval * 60)).await;
                continue;
            }
        };
        let now = Local::now().time();
        // 監視サイクル境界時刻（1 スイープ 1 値・UTC rfc3339 秒精度 Z 終端）。同一サイクルの全レースが
        // 同一 captured_at を共有し、live_ev_snapshots の「辞書順＝時刻順」「(race_id, captured_at) 冪等」
        // 契約を満たす（旧 refresh_ev.sh の `date -u +%Y-%m-%dT%H:%M:%SZ` と同表記）。
        let captured_at = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        // 発走状態は 1 スイープ 1 回だけ算出し、sweep 表示と終了判定で共有する。
        let statuses: Vec<RaceStatus> = slots
            .iter()
            .map(|s| classify(now, s.post_time, has_result(&s.race), window))
            .collect();

        // 防御: 発走前（now <= post）なのに結果取込済みのレースは、races_by_date の不変条件
        //（発走前＝race_cards 由来で track_condition=NULL）が崩れた兆候。放置すると Started 誤判定で
        // 監視が無言の no-op 化するため、検出したら警告して気づけるようにする。
        let started_before_post = slots
            .iter()
            .filter(|s| has_result(&s.race) && s.post_time.is_some_and(|p| now <= p))
            .count();
        if started_before_post > 0 {
            println!(
                "⚠ 発走前なのに結果取込済みのレースが {started_before_post} 件あります。発走状態判定の前提が \
                 崩れている可能性があり、対象から外れます（fetch-card / 成績取込の状態を確認してください）。"
            );
        }

        let ctx = SweepCtx {
            cli,
            notify_gate,
            blend_alpha,
        };
        sweep(app, &slots, &statuses, now, &captured_at, ctx).await;

        if !should_continue(&statuses) {
            if statuses.is_empty() {
                println!("── 監視終了: 本日（{}）は対象開催がありません。", cli.date);
            } else if statuses.iter().all(|s| *s == RaceStatus::Unknown) {
                println!(
                    "── 監視終了: 全レースで発走時刻（post_time）が不明です。fetch-card 済みか確認してください。"
                );
            } else {
                println!("── 監視終了: 発走前のレースが残っていません。");
            }
            break;
        }
        if cli.once {
            println!("── --once 指定のため 1 スイープで終了します。");
            break;
        }
        tokio::time::sleep(StdDuration::from_secs(cli.interval * 60)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
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

    #[test]
    fn unknown_when_no_post_time() {
        assert_eq!(
            classify(t(15, 0), None, false, Duration::minutes(40)),
            RaceStatus::Unknown
        );
    }

    #[test]
    fn started_when_result_present() {
        // 結果取込済みは発走前の時刻でも Started（再取得しない）。
        assert_eq!(
            classify(t(14, 0), Some(t(15, 0)), true, Duration::minutes(40)),
            RaceStatus::Started
        );
    }

    #[test]
    fn started_when_now_past_post() {
        assert_eq!(
            classify(t(15, 1), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Started
        );
    }

    #[test]
    fn due_within_window_inclusive_boundary() {
        // 残り 40 分ちょうどは窓内（境界を含む）。
        assert_eq!(
            classify(t(14, 20), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Due
        );
        // 残り 1 分も Due。
        assert_eq!(
            classify(t(14, 59), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Due
        );
        // 発走時刻ちょうど（残り 0 分）も発走前扱いで Due。
        assert_eq!(
            classify(t(15, 0), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::Due
        );
    }

    #[test]
    fn not_yet_when_outside_window() {
        // 残り 41 分は窓の外。
        assert_eq!(
            classify(t(14, 19), Some(t(15, 0)), false, Duration::minutes(40)),
            RaceStatus::NotYet
        );
    }

    #[test]
    fn should_continue_while_due_or_not_yet_remains() {
        use RaceStatus::*;
        // Due か NotYet が 1 つでもあれば継続。
        assert!(should_continue(&[Started, Due, Started]));
        assert!(should_continue(&[Started, NotYet]));
        // 全て発走済み or 不明なら終了。
        assert!(!should_continue(&[Started, Started, Unknown]));
        assert!(!should_continue(&[Unknown]));
        // 空（その日に開催なし）も終了。
        assert!(!should_continue(&[]));
    }
}
