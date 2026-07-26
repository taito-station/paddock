//! 予想（順位＋根拠）の CLI 表示整形（presentation アダプタ）。
//!
//! domain の `HorseProbability` / `HorseExplanation` を人間可読な行（`Vec<String>`）に写像する純関数群。
//! `paddock-predict`（対話）と `paddock-predict-watch`（ライブ監視）の両方から使うため、app から
//! interface 層へ括り出した（rest-controller が domain→HTTP を担うのと同様、domain→CLI テキストを担う）。
//! `println!` 副作用は各 app 側に残し、ここは整形のみ（テスト容易性のため）。

use std::collections::HashMap;

use paddock_domain::{
    BetMethod, ExplainCategory, FactorExplanation, HorseExplanation, HorseNum, HorseProbability,
    Portfolio, PrevRunSummary, Surface, Verdict,
};

/// 確率テーブル（馬番/馬名/勝率/連対率/複勝率）を盤面順のまま行に整形する。先頭はヘッダ行。
pub fn format_probs(probs: &[HorseProbability]) -> Vec<String> {
    let mut lines = vec![format!(
        "{:<4} {:<16} {:>8} {:>8} {:>8}",
        "馬番", "馬名", "勝率", "連対率", "複勝率"
    )];
    for p in probs {
        lines.push(format!(
            "{:>4} {:<16} {:>7.1}% {:>7.1}% {:>7.1}%",
            p.horse_num.value(),
            p.horse_name.value(),
            p.win_prob * 100.0,
            p.place_prob * 100.0,
            p.show_prob * 100.0,
        ));
    }
    lines
}

/// 近走データ（`horse_past_runs`）欠損の警告行を組む（#552）。新馬戦（構造上ゼロ）・近走取得全滅の
/// いずれも近走フォーム特徴量が欠損し、確率・買い目・回収率の信頼性が下がる。回収率だけを見て候補入り
/// してしまうのを防ぐ注記が目的で、表示自体は従来どおり続ける（呼び出し側が確率テーブルの前に出す）。
///
/// `field_size` は出走頭数、`horses_with_runs` は近走を 1 走以上持つ頭数。判定閾値は
/// **全頭ゼロ**（より強い文言）と **過半で欠損**（Issue #552 の提案どおり）。どちらでもなければ `None`。
/// 新馬戦か取得失敗かは DB 状態だけでは確実に区別できないため、文言は両者を併記する。
pub fn format_recent_runs_warning(field_size: usize, horses_with_runs: usize) -> Option<String> {
    if field_size == 0 {
        return None;
    }
    let without = field_size.saturating_sub(horses_with_runs);
    if horses_with_runs == 0 {
        Some(format!(
            "⚠️ 近走データ皆無（新馬戦/近走取得失敗）: 全 {field_size} 頭が近走ゼロ。\
             確率・買い目・回収率の信頼性は極めて低い（回収率だけで候補入り判断しない）。"
        ))
    } else if without * 2 > field_size {
        Some(format!(
            "⚠️ 近走データ欠損: 全 {field_size} 頭中 {without} 頭が近走ゼロ（新馬戦/近走取得失敗）。\
             確率・回収率の信頼性は低い。"
        ))
    } else {
        None
    }
}

/// 条件依存枠バイアスの複勝 lift がこの値以上で「枠有利」とみなし、市場過小評価と重なれば枠妙味を光らせる
/// 閾値（#343）。lift は複勝ベース・市場差分は単勝ベースの近似併用（下記 doc 参照）。
/// TODO(#343 後続): この 0.05 は measure-first の暫定値。backtest lift 掃引（`--gate-bias-weight` 相当）で
/// 校正してから確定する（現段階は提示のみ・スコア非投入なので回収率に影響しない）。
const GATE_BIAS_FLAG_LIFT: f64 = 0.05;

/// 過去データ視点の比較テーブル（#272 ④）。純モデル勝率と市場 implied 勝率を並べ、差（pt）を見せる。
/// `market_win` は馬番→単勝オッズ（払戻倍率 ≥1.0）。implied = `(1/odds)/Σ(1/odds)`（控除率＝オーバー
/// ラウンドを除去）。オッズの無い馬は市場・差欄を `-` にする。盤面順（`pure` の順）で出力し先頭はヘッダ。
/// 差 = 純勝率 − 市場implied（正＝モデルが市場より強気）。EV の向き（割安/割高）を読む材料。
///
/// `gate_lift` は馬番→条件依存枠バイアスの複勝 lift（#343）。**枠有利（lift≥[`GATE_BIAS_FLAG_LIFT`]）かつ
/// 市場過小（純勝率 > 市場implied）** の馬に `🔶枠妙味` を付す＝「市場が見落としている枠バイアス」だけを
/// 光らせる（decision-support・提示のみ。軸は動かさない, ADR 0055/0060）。lift は複勝ベース・市場差分は
/// 単勝ベースの近似併用（複勝の市場implied は place odds 依存で広く取れないため単勝差分を代理に使う）。
/// フラグ不要な経路は空 map を渡す。
pub fn format_probs_with_market(
    pure: &[HorseProbability],
    market_win: &HashMap<HorseNum, f64>,
    gate_lift: &HashMap<HorseNum, f64>,
) -> Vec<String> {
    // 控除率除去のため、有効オッズの 1/odds 合計で正規化する（blend_with_market_win と同じ implied 定義）。
    let overround: f64 = market_win
        .values()
        .filter(|&&o| o.is_finite() && o >= 1.0)
        .map(|&o| 1.0 / o)
        .sum();
    let mut lines = vec![format!(
        "{:<4} {:<16} {:>8} {:>8} {:>8}",
        "馬番", "馬名", "純勝率", "市場", "差pt"
    )];
    for p in pure {
        let implied = market_win
            .get(&p.horse_num)
            .filter(|&&o| o.is_finite() && o >= 1.0 && overround > 0.0)
            .map(|&o| (1.0 / o) / overround);
        let (mkt, diff) = match implied {
            Some(m) => (
                format!("{:>7.1}%", m * 100.0),
                format!("{:>+8.1}", (p.win_prob - m) * 100.0),
            ),
            None => (format!("{:>8}", "-"), format!("{:>8}", "-")),
        };
        // 枠妙味フラグ: 枠有利（lift≥閾値）かつ市場過小（純勝率 > 市場implied）だけを光らせる（#343）。
        let gate_edge = match implied {
            Some(m) => {
                gate_lift
                    .get(&p.horse_num)
                    .is_some_and(|&l| l >= GATE_BIAS_FLAG_LIFT)
                    && p.win_prob > m
            }
            None => false,
        };
        lines.push(format!(
            "{:>4} {:<16} {:>7.1}% {} {}{}",
            p.horse_num.value(),
            p.horse_name.value(),
            p.win_prob * 100.0,
            mkt,
            diff,
            if gate_edge { "  🔶枠妙味" } else { "" },
        ));
    }
    lines
}

/// win_prob 上位の馬について予想根拠（条件別成績・近走フォーム・前走・斤量）を印付きで整形し、
/// 表示行を順に返す（#274）。確率テーブルは盤面順なので win_prob 降順に並べ替えて上位 `MARKS` 頭に
/// 印を振る。`println!` から分離して純粋関数にし、ランク付け・印・フォールバックをテスト可能にする。
pub fn format_explanations(
    probs: &[HorseProbability],
    explanations: &[HorseExplanation],
) -> Vec<String> {
    const MARKS: [&str; 5] = ["◎", "○", "▲", "△", "☆"];
    let mut ranked: Vec<&HorseProbability> = probs.iter().collect();
    ranked.sort_by(|a, b| {
        b.win_prob
            .partial_cmp(&a.win_prob)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 馬番→根拠の引き当てを O(1) にする（並べ替えで probs/explanations の位置対応が崩れるため馬番で突き合わせる）。
    let by_num: HashMap<HorseNum, &HorseExplanation> =
        explanations.iter().map(|e| (e.horse_num, e)).collect();

    let shown = ranked.len().min(MARKS.len());
    let mut lines = vec![format!("【予想根拠（上位{shown}頭）】")];
    for (rank, p) in ranked.into_iter().take(MARKS.len()).enumerate() {
        let mark = MARKS[rank];
        let Some(ex) = by_num.get(&p.horse_num) else {
            continue;
        };
        lines.push(format!(
            "{mark}{} {}（勝率{:.1}%）",
            p.horse_num.value(),
            p.horse_name.value(),
            p.win_prob * 100.0,
        ));
        // この馬の根拠本文（factor / 近走フォーム / 前走 / 斤量）。1 行も無ければデータ不足とする。
        let mut body: Vec<String> = Vec::new();
        for f in &ex.factors {
            body.push(factor_phrase(f));
        }
        if let Some(form) = ex.recent_form {
            body.push(recent_form_phrase(form));
        }
        if let Some(prev) = &ex.prev_run {
            body.push(prev_run_phrase(prev));
        }
        if let (Some(w), Some(mean)) = (ex.weight_carried, ex.field_mean_weight) {
            body.push(format!("斤量 {w:.1}kg（平均比 {:+.1}kg）", w - mean));
        }
        if body.is_empty() {
            body.push("（実績データ不足）".to_string());
        }
        lines.extend(body.into_iter().map(|b| format!("  {b}")));
    }
    lines
}

/// 1 factor 分の根拠を 1 行の日本語にする。カテゴリで話題語、`verdict` があれば「得意/標準/苦手」を付ける。
/// `verdict == None`（CourseGate＝場×枠の全馬横断率）は率だけ提示する（馬の適性ではないため誤読防止）。
fn factor_phrase(f: &FactorExplanation) -> String {
    let topic = match f.category {
        ExplainCategory::Surface | ExplainCategory::Distance => f.label.clone(),
        ExplainCategory::TrackCondition => format!("{}馬場", f.label),
        ExplainCategory::CourseGate => format!("枠（{}）", gate_label_jp(&f.label)),
        // 条件依存枠バイアス（#343）。label は use-case で「内枠 / 良 / 多(14-18)」形式に整形済み。
        ExplainCategory::ConditionalGateBias => format!("枠バイアス（{}）", f.label),
        ExplainCategory::Jockey => format!("騎手 {}", f.label),
        ExplainCategory::Trainer => format!("厩舎 {}", f.label),
        // 相性 factor（#366(b)・率のみ）。board 書評 commentary::factor_topic と話題語を揃える。
        ExplainCategory::JockeyVenue | ExplainCategory::JockeyDistance => {
            format!("騎手の{}成績", f.label)
        }
        ExplainCategory::JockeyHorseCombo => format!("馬×騎手（{}）", f.label),
        ExplainCategory::HorseVenue => format!("当場（{}）", f.label),
    };
    match f.verdict {
        Some(v) => {
            let word = match v {
                Verdict::Strong => "得意",
                Verdict::Neutral => "標準",
                Verdict::Weak => "苦手",
            };
            format!(
                "{topic} {word}：複勝率 {:.0}%（{}走）",
                f.rate.show * 100.0,
                f.starts,
            )
        }
        None => format!(
            "{topic}：複勝率 {:.0}%（{}走）",
            f.rate.show * 100.0,
            f.starts
        ),
    }
}

/// 枠グループラベル（コース統計のキー由来の英語。use-case の `gate_group_label` が生成）を日本語
/// 表記に写像する（#274 レビュー）。ラベルは統計キーで英語固定のため、表示は presentation 層で日本語化する。
fn gate_label_jp(label: &str) -> &str {
    match label {
        "Inner (1-3)" => "内 1-3",
        "Middle (4-6)" => "中 4-6",
        "Outer (7-8)" => "外 7-8",
        other => other, // 想定外ラベルはそのまま（将来キー書式が変わっても壊さない）
    }
}

/// 近走フォームスコア [0,1]（0.5=中立）を「好調/標準/不調」の 1 行にする（#274）。
/// 馬体重変化・人気乖離・間隔・着差・タイムを合成した近走の勢いの要約（`config.trend_n` 走、本番は
/// 前走のみ）で、前走の着順などの具体（[`prev_run_phrase`]）とは別軸の signal。
fn recent_form_phrase(form: f64) -> String {
    let label = if form >= 0.6 {
        "好調"
    } else if form <= 0.4 {
        "不調"
    } else {
        "標準"
    };
    format!("近走フォーム：{label}（{form:.2}）")
}

/// 前走サマリを 1 行の日本語にする。欠落フィールドは黙って省く。
fn prev_run_phrase(p: &PrevRunSummary) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pos) = p.finishing_position {
        parts.push(format!("{pos}着"));
    }
    if let Some(pop) = p.popularity {
        parts.push(format!("{pop}番人気"));
    }
    parts.push(format!("{}{}m", surface_jp(p.surface), p.distance));
    if let Some(m) = &p.margin
        && !m.is_empty()
    {
        parts.push(format!("着差{m}"));
    }
    format!("前走：{}", parts.join("・"))
}

/// 馬場種別（芝/ダート）の日本語表記。レースヘッダ・前走サマリの双方で使う。
pub fn surface_jp(s: Surface) -> &'static str {
    match s {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    }
}

/// [`format_portfolio`] の app 別表示オプション（#452）。買い目本体（軸/相手・混戦注記・各点）の整形は
/// 共有しつつ、app 固有の見た目差だけをここで吸収する。predict（対話）と predict-watch（ライブ監視）で
/// インデント幅・0 円脚の扱い・未取得脚の EV 表示が異なるため、その差を明示的なフラグとして持つ。
#[derive(Debug, Clone, Copy)]
pub struct PortfolioFormat {
    /// 各行頭のインデント（predict は 2 スペース、predict-watch は 5 スペース）。
    pub indent: &'static str,
    /// `stake == 0` の脚を出力から落とすか（predict-watch は落とす。predict は 0 円脚を出さない
    /// 前提で全脚を出す＝現行挙動を保つ）。
    pub skip_zero_stake: bool,
    /// オッズ未取得（`odds == None`）の脚に `EV={:.2}` を付けるか（predict は付ける、
    /// predict-watch は付けない＝現行の各 app 出力をバイト単位で保つ）。
    pub ev_on_unpriced: bool,
}

/// ポートフォリオを「そのまま買える形」（CLAUDE.md 表記規約: 方式/軸/相手/各点=式別×金額）に整形し、
/// 行の羅列（`Vec<String>`）で返す（#452）。predict と predict-watch のほぼ同一だったインライン整形を
/// 一本化したもの。ヘッダ（券種予算行）・フッタ（賭け計 / 期待回収率）や軸なし・買い目なしの注記は
/// app 側で前後に付ける（それらは app 固有で重複していないため共有しない）。
///
/// 出力する行:
/// 1. 軸行 `軸 {axis} → 相手 {p1},{p2},...`（`axis` が `Some` のときのみ）
/// 2. 混戦注記 `混戦: 印馬3連複ボックス（軸なし）を併用`（Box 方式の脚が 1 つでもあるとき）
/// 3. 各買い目行 `[{方式}] {組合せ} ¥{金額} {オッズ|未取得} 的中{:.1}%[ EV={:.2}]`
///
/// 方式は `ながし`（軸流し）/ `ボックス`（軸なし総当たり）を明示し、CLAUDE.md の「ながし/ボックス/
/// フォーメーションを正しく区別する」表記規約に従う。金額は domain 側で 100 円単位に整えられている。
pub fn format_portfolio(p: &Portfolio, fmt: &PortfolioFormat) -> Vec<String> {
    let indent = fmt.indent;
    let mut lines = Vec::new();
    if let Some(axis) = p.axis {
        let partners = p
            .partners
            .iter()
            .map(|h| h.value().to_string())
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!("{indent}軸 {} → 相手 {}", axis.value(), partners));
    }
    // 方式（ながし/ボックス）を明示する。box は軸を持たない印馬総当たりで、「軸流し」枠の脚と
    // 混同しないよう区別表示する（CLAUDE.md 表記規約）。
    // 混戦注記は `Portfolio.konsen` フラグではなく Box 脚の有無で判定する。旧 predict/predict-watch
    // 両実装がこの条件だったため、出力をバイト単位で保つべく踏襲する（混戦でも予算不足で Box 脚が
    // 0 点なら注記を出さない、という現行挙動を変えない。konsen へ差し替えないこと）。
    if p.bets.iter().any(|b| b.method == BetMethod::Box) {
        lines.push(format!("{indent}混戦: 印馬3連複ボックス（軸なし）を併用"));
    }
    for bet in &p.bets {
        if fmt.skip_zero_stake && bet.stake == 0 {
            continue;
        }
        let method = match bet.method {
            BetMethod::Nagashi => "ながし",
            BetMethod::Box => "ボックス",
        };
        let label = bet.combination.label_ja();
        match bet.odds {
            Some(o) => lines.push(format!(
                "{indent}[{}] {} ¥{} オッズ{:.1} 的中{:.1}% EV={:.2}",
                method,
                label,
                bet.stake,
                o,
                bet.hit_prob * 100.0,
                bet.ev,
            )),
            None if fmt.ev_on_unpriced => lines.push(format!(
                "{indent}[{}] {} ¥{} オッズ未取得 的中{:.1}% EV={:.2}",
                method,
                label,
                bet.stake,
                bet.hit_prob * 100.0,
                bet.ev,
            )),
            None => lines.push(format!(
                "{indent}[{}] {} ¥{} オッズ未取得 的中{:.1}%",
                method,
                label,
                bet.stake,
                bet.hit_prob * 100.0,
            )),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{
        factor_phrase, format_explanations, format_probs, format_recent_runs_warning,
        gate_label_jp, prev_run_phrase, recent_form_phrase, surface_jp,
    };
    use paddock_domain::horse_result::HorseNum;
    use paddock_domain::{
        ExplainCategory, FactorExplanation, HorseExplanation, HorseProbability, PrevRunSummary,
        RateTriple, Surface, Verdict,
    };

    fn horse(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    fn factor(
        category: ExplainCategory,
        label: &str,
        show: f64,
        starts: u32,
        verdict: Option<Verdict>,
    ) -> FactorExplanation {
        FactorExplanation {
            category,
            label: label.to_string(),
            rate: RateTriple {
                win: show / 3.0,
                place: show * 2.0 / 3.0,
                show,
            },
            starts,
            verdict,
        }
    }

    fn prob(num: u32, name: &str, win: f64) -> HorseProbability {
        HorseProbability {
            horse_num: horse(num),
            horse_name: paddock_domain::horse_result::HorseName::try_from(name).unwrap(),
            win_prob: win,
            place_prob: win,
            show_prob: win,
        }
    }

    fn explanation(num: u32, name: &str, factors: Vec<FactorExplanation>) -> HorseExplanation {
        HorseExplanation {
            horse_num: horse(num),
            horse_name: paddock_domain::horse_result::HorseName::try_from(name).unwrap(),
            factors,
            recent_form: None,
            prev_run: None,
            gate_bias_lift: None,
            weight_carried: None,
            field_mean_weight: None,
        }
    }

    #[test]
    fn recent_runs_warning_all_missing_is_strong() {
        // 全頭ゼロ（新馬戦/取得全滅）は「皆無」の強い文言。
        let w = format_recent_runs_warning(16, 0).unwrap();
        assert!(w.contains("皆無"), "全欠損は皆無文言: {w}");
        assert!(w.contains("全 16 頭"));
    }

    #[test]
    fn recent_runs_warning_majority_missing() {
        // 過半（16頭中9頭欠損）で警告。文言に欠損頭数を含む。
        let w = format_recent_runs_warning(16, 7).unwrap();
        assert!(w.contains("欠損"), "過半欠損は欠損文言: {w}");
        assert!(w.contains("9 頭が近走ゼロ"), "欠損頭数を明示: {w}");
    }

    #[test]
    fn recent_runs_warning_exactly_half_is_none() {
        // 半数ちょうど（過半ではない）は警告しない（without*2 > field で判定）。
        assert_eq!(format_recent_runs_warning(16, 8), None);
    }

    #[test]
    fn recent_runs_warning_minority_missing_is_none() {
        // 欠損が少数（16頭中1頭）は通常レース扱いで警告なし。
        assert_eq!(format_recent_runs_warning(16, 15), None);
    }

    #[test]
    fn recent_runs_warning_full_coverage_is_none() {
        assert_eq!(format_recent_runs_warning(12, 12), None);
    }

    #[test]
    fn recent_runs_warning_zero_field_is_none() {
        // 出走頭数 0 は判定対象外（0除算・偽陽性を避ける）。
        assert_eq!(format_recent_runs_warning(0, 0), None);
    }

    #[test]
    fn factor_phrase_renders_verdict_for_horse_factors() {
        let f = factor(
            ExplainCategory::Surface,
            "芝",
            0.5,
            20,
            Some(Verdict::Strong),
        );
        assert_eq!(factor_phrase(&f), "芝 得意：複勝率 50%（20走）");
        let f = factor(
            ExplainCategory::TrackCondition,
            "重",
            0.0,
            5,
            Some(Verdict::Weak),
        );
        assert_eq!(factor_phrase(&f), "重馬場 苦手：複勝率 0%（5走）");
    }

    #[test]
    fn factor_phrase_omits_verdict_and_jp_label_for_course_gate() {
        // 枠は全馬横断のベース率なので得意/苦手を出さず（verdict None）、ラベルは日本語化する（#274 レビュー）。
        let f = factor(ExplainCategory::CourseGate, "Outer (7-8)", 0.23, 622, None);
        assert_eq!(factor_phrase(&f), "枠（外 7-8）：複勝率 23%（622走）");
    }

    #[test]
    fn factor_phrase_omits_verdict_for_jockey_and_trainer() {
        // 騎手・調教師は馬の適性ではないため verdict なし（率のみ, #274 レビュー）。
        let j = factor(ExplainCategory::Jockey, "ルメール", 0.4, 100, None);
        assert_eq!(factor_phrase(&j), "騎手 ルメール：複勝率 40%（100走）");
        let t = factor(ExplainCategory::Trainer, "藤沢", 0.3, 80, None);
        assert_eq!(factor_phrase(&t), "厩舎 藤沢：複勝率 30%（80走）");
    }

    #[test]
    fn factor_phrase_renders_affinity_factors() {
        // #366(b) 相性 factor は率のみ提示（verdict なし）。board 書評 commentary::factor_topic と
        // 話題語を揃える（2 formatter の乖離検知）。
        let jv = factor(ExplainCategory::JockeyVenue, "函館", 0.28, 40, None);
        assert_eq!(factor_phrase(&jv), "騎手の函館成績：複勝率 28%（40走）");
        let jd = factor(
            ExplainCategory::JockeyDistance,
            "1500〜1800m",
            0.3,
            50,
            None,
        );
        assert_eq!(
            factor_phrase(&jd),
            "騎手の1500〜1800m成績：複勝率 30%（50走）"
        );
        let combo = factor(ExplainCategory::JockeyHorseCombo, "武豊", 0.5, 8, None);
        assert_eq!(factor_phrase(&combo), "馬×騎手（武豊）：複勝率 50%（8走）");
        let hv = factor(ExplainCategory::HorseVenue, "函館", 0.33, 6, None);
        assert_eq!(factor_phrase(&hv), "当場（函館）：複勝率 33%（6走）");
    }

    #[test]
    fn gate_label_jp_maps_all_groups_and_passes_through_unknown() {
        assert_eq!(gate_label_jp("Inner (1-3)"), "内 1-3");
        assert_eq!(gate_label_jp("Middle (4-6)"), "中 4-6");
        assert_eq!(gate_label_jp("Outer (7-8)"), "外 7-8");
        // 想定外ラベルは素通し（domain 側書式変更で壊さない）。
        assert_eq!(gate_label_jp("???"), "???");
    }

    #[test]
    fn recent_form_phrase_buckets_by_score() {
        assert_eq!(recent_form_phrase(0.72), "近走フォーム：好調（0.72）");
        assert_eq!(recent_form_phrase(0.50), "近走フォーム：標準（0.50）");
        assert_eq!(recent_form_phrase(0.30), "近走フォーム：不調（0.30）");
    }

    #[test]
    fn recent_form_phrase_boundaries_are_inclusive() {
        // 境界: >=0.6 は好調・<=0.4 は不調（等号を含む）。中間は標準。
        assert_eq!(recent_form_phrase(0.60), "近走フォーム：好調（0.60）");
        assert_eq!(recent_form_phrase(0.40), "近走フォーム：不調（0.40）");
        assert_eq!(recent_form_phrase(0.59), "近走フォーム：標準（0.59）");
        assert_eq!(recent_form_phrase(0.41), "近走フォーム：標準（0.41）");
    }

    #[test]
    fn format_explanations_ranks_marks_and_matches_by_horse_num() {
        // probs は盤面順（馬番昇順）で勝率は逆順。format_explanations は勝率降順に並べ替えて印を振る。
        let probs = vec![
            prob(1, "ウマ1", 0.10),
            prob(2, "ウマ2", 0.50),
            prob(3, "ウマ3", 0.30),
        ];
        // explanations の順序は probs と別（馬番で引き当てられることの確認）。
        let expls = vec![
            explanation(
                3,
                "ウマ3",
                vec![factor(
                    ExplainCategory::Surface,
                    "芝",
                    0.5,
                    20,
                    Some(Verdict::Strong),
                )],
            ),
            explanation(2, "ウマ2", vec![]), // factor 無し → データ不足
            explanation(1, "ウマ1", vec![]),
        ];
        let lines = format_explanations(&probs, &expls);
        assert_eq!(lines[0], "【予想根拠（上位3頭）】");
        // 勝率降順: ◎ウマ2(0.50) → ○ウマ3(0.30) → ▲ウマ1(0.10)
        assert_eq!(lines[1], "◎2 ウマ2（勝率50.0%）");
        assert_eq!(lines[2], "  （実績データ不足）");
        assert_eq!(lines[3], "○3 ウマ3（勝率30.0%）");
        assert_eq!(lines[4], "  芝 得意：複勝率 50%（20走）");
        assert_eq!(lines[5], "▲1 ウマ1（勝率10.0%）");
        assert_eq!(lines[6], "  （実績データ不足）");
    }

    #[test]
    fn format_explanations_weight_only_is_not_data_insufficient() {
        // factor・前走が無くても斤量があれば「データ不足」にしない（#274 レビュー C10）。
        let probs = vec![prob(1, "ウマ1", 0.2)];
        let mut ex = explanation(1, "ウマ1", vec![]);
        ex.weight_carried = Some(57.0);
        ex.field_mean_weight = Some(55.0);
        let lines = format_explanations(&probs, &[ex]);
        assert_eq!(lines[1], "◎1 ウマ1（勝率20.0%）");
        assert_eq!(lines[2], "  斤量 57.0kg（平均比 +2.0kg）");
        assert!(!lines.iter().any(|l| l.contains("実績データ不足")));
    }

    #[test]
    fn prev_run_phrase_omits_missing_fields() {
        let full = PrevRunSummary {
            finishing_position: Some(3),
            popularity: Some(8),
            margin: Some("クビ".to_string()),
            surface: Surface::Turf,
            distance: 1600,
        };
        assert_eq!(
            prev_run_phrase(&full),
            "前走：3着・8番人気・芝1600m・着差クビ"
        );

        // 着順・人気・着差が欠落（中止等）でもコースは出る。
        let sparse = PrevRunSummary {
            finishing_position: None,
            popularity: None,
            margin: None,
            surface: Surface::Dirt,
            distance: 1800,
        };
        assert_eq!(prev_run_phrase(&sparse), "前走：ダート1800m");
    }

    #[test]
    fn format_probs_renders_header_and_rows() {
        // 先頭はヘッダ行、以降は盤面順 1 頭 1 行。率は小数 1 桁＋%（prob は win=place=show）。
        let probs = vec![prob(7, "ウマ7", 0.123), prob(3, "ウマ3", 0.5)];
        let lines = format_probs(&probs);
        assert_eq!(lines.len(), 3);
        assert!(
            lines[0].contains("馬番") && lines[0].contains("勝率") && lines[0].contains("複勝率")
        );
        assert!(lines[1].contains("ウマ7") && lines[1].contains("12.3%"));
        assert!(lines[2].contains("ウマ3") && lines[2].contains("50.0%"));
    }

    #[test]
    fn surface_jp_maps_both_surfaces() {
        assert_eq!(surface_jp(Surface::Turf), "芝");
        assert_eq!(surface_jp(Surface::Dirt), "ダート");
    }

    // --- format_portfolio（#452） ---
    use super::{PortfolioFormat, format_portfolio};
    use paddock_domain::{BetCombination, BetMethod, Pair, Portfolio, PortfolioBet, Triple};

    fn pf_bet(
        combo: BetCombination,
        method: BetMethod,
        stake: u64,
        odds: Option<f64>,
    ) -> PortfolioBet {
        PortfolioBet {
            combination: combo,
            method,
            stake,
            odds,
            ev: 1.23,
            hit_prob: 0.25,
        }
    }

    /// predict 側の設定（2 スペース・0 円脚も出す・未取得脚にも EV）。
    fn predict_fmt() -> PortfolioFormat {
        PortfolioFormat {
            indent: "  ",
            skip_zero_stake: false,
            ev_on_unpriced: true,
        }
    }

    /// predict-watch 側の設定（5 スペース・0 円脚を落とす・未取得脚に EV なし）。
    fn watch_fmt() -> PortfolioFormat {
        PortfolioFormat {
            indent: "     ",
            skip_zero_stake: true,
            ev_on_unpriced: false,
        }
    }

    #[test]
    fn format_portfolio_predict_axis_partners_and_priced_leg() {
        let quinella = BetCombination::Quinella(Pair::try_from((horse(1), horse(5))).unwrap());
        let pf = Portfolio {
            axis: Some(horse(1)),
            partners: vec![horse(5), horse(3)],
            konsen: false,
            bets: vec![pf_bet(quinella, BetMethod::Nagashi, 300, Some(4.2))],
            total_stake: 300,
            ev: None,
        };
        let lines = format_portfolio(&pf, &predict_fmt());
        assert_eq!(lines[0], "  軸 1 → 相手 5,3");
        // priced 脚: [方式] 組合せ ¥金額 オッズX.X 的中Y% EV=Z（predict/watch 共通の priced 書式）。
        assert_eq!(
            lines[1],
            "  [ながし] 馬連 1-5 ¥300 オッズ4.2 的中25.0% EV=1.23"
        );
        assert_eq!(lines.len(), 2, "軸なし・空注記・フッタは app 側で付す");
    }

    #[test]
    fn format_portfolio_konsen_note_and_box_method() {
        let trio = BetCombination::Trio(Triple::try_from((horse(1), horse(2), horse(3))).unwrap());
        let pf = Portfolio {
            axis: Some(horse(1)),
            partners: vec![horse(2)],
            konsen: true,
            bets: vec![pf_bet(trio, BetMethod::Box, 100, Some(30.0))],
            total_stake: 100,
            ev: None,
        };
        let lines = format_portfolio(&pf, &predict_fmt());
        assert_eq!(lines[0], "  軸 1 → 相手 2");
        // Box 脚が 1 つでもあれば混戦注記を出す（ながし/ボックスの区別＝表記規約）。
        assert_eq!(lines[1], "  混戦: 印馬3連複ボックス（軸なし）を併用");
        assert_eq!(
            lines[2],
            "  [ボックス] 三連複 1-2-3 ¥100 オッズ30.0 的中25.0% EV=1.23"
        );
    }

    #[test]
    fn format_portfolio_unpriced_ev_toggle_differs_by_app() {
        let quinella = BetCombination::Quinella(Pair::try_from((horse(1), horse(4))).unwrap());
        let pf = Portfolio {
            axis: Some(horse(1)),
            partners: vec![horse(4)],
            konsen: false,
            bets: vec![pf_bet(quinella, BetMethod::Nagashi, 200, None)],
            total_stake: 200,
            ev: None,
        };
        // predict: 未取得脚にも EV を付ける。
        let predict = format_portfolio(&pf, &predict_fmt());
        assert_eq!(
            predict[1],
            "  [ながし] 馬連 1-4 ¥200 オッズ未取得 的中25.0% EV=1.23"
        );
        // predict-watch: 未取得脚は EV を付けず 5 スペースインデント。
        let watch = format_portfolio(&pf, &watch_fmt());
        assert_eq!(
            watch[1],
            "     [ながし] 馬連 1-4 ¥200 オッズ未取得 的中25.0%"
        );
    }

    #[test]
    fn format_portfolio_watch_skips_zero_stake_predict_keeps_it() {
        let a = BetCombination::Quinella(Pair::try_from((horse(1), horse(2))).unwrap());
        let b = BetCombination::Quinella(Pair::try_from((horse(1), horse(3))).unwrap());
        let pf = Portfolio {
            axis: Some(horse(1)),
            partners: vec![horse(2), horse(3)],
            konsen: false,
            bets: vec![
                pf_bet(a, BetMethod::Nagashi, 0, Some(5.0)),
                pf_bet(b, BetMethod::Nagashi, 100, Some(5.0)),
            ],
            total_stake: 100,
            ev: None,
        };
        // predict は 0 円脚も出す（2 脚）。
        let predict = format_portfolio(&pf, &predict_fmt());
        assert_eq!(predict.len(), 3, "軸行 + 2 脚");
        // predict-watch は 0 円脚を落とす（1 脚だけ）。
        let watch = format_portfolio(&pf, &watch_fmt());
        assert_eq!(watch.len(), 2, "軸行 + 100 円脚のみ");
        assert!(watch[1].contains("¥100"));
    }

    #[test]
    fn format_portfolio_no_axis_emits_no_lines() {
        // 軸 None（確率推定が空）は共有整形からは何も出さない（app 側が注記を付す）。
        let pf = Portfolio {
            axis: None,
            partners: vec![],
            konsen: false,
            bets: vec![],
            total_stake: 0,
            ev: None,
        };
        assert!(format_portfolio(&pf, &predict_fmt()).is_empty());
        assert!(format_portfolio(&pf, &watch_fmt()).is_empty());
    }

    #[test]
    fn format_probs_with_market_shows_pure_market_and_diff() {
        use super::format_probs_with_market;
        use std::collections::HashMap;
        let pure = vec![prob(1, "ウマ1", 0.30), prob(2, "ウマ2", 0.10)];
        // 馬1 odds=2.0→1/2=0.5、馬2 odds=4.0→0.25。overround=0.75。
        // implied: 馬1=0.5/0.75≒66.7%、馬2=0.25/0.75≒33.3%。
        let mut market: HashMap<HorseNum, f64> = HashMap::new();
        market.insert(horse(1), 2.0);
        market.insert(horse(2), 4.0);
        let lines = format_probs_with_market(&pure, &market, &HashMap::new());
        assert!(
            lines[0].contains("純勝率") && lines[0].contains("市場") && lines[0].contains("差pt")
        );
        // 馬1: 純30.0% 市場66.7% 差 -36.7（純<市場＝モデルは市場より弱気）。
        assert!(
            lines[1].contains("ウマ1")
                && lines[1].contains("30.0%")
                && lines[1].contains("66.7%")
                && lines[1].contains("-36.7")
        );
        // 馬2: 市場33.3%。
        assert!(lines[2].contains("ウマ2") && lines[2].contains("33.3%"));
    }

    #[test]
    fn format_probs_with_market_flags_gate_edge_when_favorable_and_underpriced() {
        use super::format_probs_with_market;
        use std::collections::HashMap;
        // 馬1: odds 5.0→implied 低め・純30%>市場 で過小、枠 lift 有利 → 🔶枠妙味。
        // 馬2: 枠 lift 有利だが odds 1.5→implied 高め・純10%<市場 で過大 → フラグなし。
        let pure = vec![prob(1, "ウマ1", 0.30), prob(2, "ウマ2", 0.10)];
        let mut market: HashMap<HorseNum, f64> = HashMap::new();
        market.insert(horse(1), 5.0);
        market.insert(horse(2), 1.5);
        let mut gate_lift: HashMap<HorseNum, f64> = HashMap::new();
        gate_lift.insert(horse(1), 0.08);
        gate_lift.insert(horse(2), 0.08);
        let lines = format_probs_with_market(&pure, &market, &gate_lift);
        assert!(
            lines[1].contains("🔶枠妙味"),
            "馬1=枠有利∧市場過小: {}",
            lines[1]
        );
        assert!(
            !lines[2].contains("🔶枠妙味"),
            "馬2=市場過大なので光らせない: {}",
            lines[2]
        );
    }

    #[test]
    fn format_probs_with_market_no_flag_when_lift_below_threshold() {
        use super::format_probs_with_market;
        use std::collections::HashMap;
        // 過小評価だが枠 lift が閾値未満 → フラグなし（枠の見落としではない）。
        let pure = vec![prob(1, "ウマ1", 0.30)];
        let mut market: HashMap<HorseNum, f64> = HashMap::new();
        market.insert(horse(1), 5.0);
        let mut gate_lift: HashMap<HorseNum, f64> = HashMap::new();
        gate_lift.insert(horse(1), 0.01);
        let lines = format_probs_with_market(&pure, &market, &gate_lift);
        assert!(!lines[1].contains("🔶枠妙味"), "{}", lines[1]);
    }

    #[test]
    fn conditional_gate_bias_factor_renders_as_gate_bias_phrase() {
        use paddock_domain::{ExplainCategory, FactorExplanation, RateTriple};
        let probs = vec![prob(1, "ウマ1", 0.30)];
        let ex = explanation(
            1,
            "ウマ1",
            vec![FactorExplanation::new(
                ExplainCategory::ConditionalGateBias,
                "内枠 / 良 / 多(14-18)".to_string(),
                RateTriple {
                    win: 0.1,
                    place: 0.2,
                    show: 0.35,
                },
                40,
            )],
        );
        let joined = format_explanations(&probs, &[ex]).join("\n");
        assert!(
            joined.contains("枠バイアス（内枠 / 良 / 多(14-18)）"),
            "{joined}"
        );
        // verdict なし（全馬横断率）＝「得意/苦手」を付けず複勝率だけ。
        assert!(joined.contains("複勝率 35%"), "{joined}");
    }

    #[test]
    fn format_probs_with_market_dashes_when_odds_missing() {
        use super::format_probs_with_market;
        use std::collections::HashMap;
        let pure = vec![prob(1, "ウマ1", 0.30)];
        let market: HashMap<HorseNum, f64> = HashMap::new(); // オッズ無し → 市場・差は「-」
        let lines = format_probs_with_market(&pure, &market, &HashMap::new());
        assert!(lines[1].contains("ウマ1") && lines[1].contains("30.0%"));
        assert!(
            lines[1].contains('-'),
            "オッズ無しは市場・差欄が「-」: {}",
            lines[1]
        );
    }
}
