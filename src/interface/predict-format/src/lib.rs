//! 予想（順位＋根拠）の CLI 表示整形（presentation アダプタ）。
//!
//! domain の `HorseProbability` / `HorseExplanation` を人間可読な行（`Vec<String>`）に写像する純関数群。
//! `paddock-predict`（対話）と `paddock-predict-watch`（ライブ監視）の両方から使うため、app から
//! interface 層へ括り出した（rest-controller が domain→HTTP を担うのと同様、domain→CLI テキストを担う）。
//! `println!` 副作用は各 app 側に残し、ここは整形のみ（テスト容易性のため）。

use std::collections::HashMap;

use paddock_domain::{
    ExplainCategory, FactorExplanation, HorseExplanation, HorseNum, HorseProbability,
    PrevRunSummary, Surface, Verdict,
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

/// 条件依存枠バイアスの複勝 lift がこの値以上で「枠有利」とみなし、市場過小評価と重なれば枠妙味を光らせる
/// 閾値（#343）。lift は複勝ベース・市場差分は単勝ベースの近似併用（下記 doc 参照）。
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

#[cfg(test)]
mod tests {
    use super::{
        factor_phrase, format_explanations, format_probs, gate_label_jp, prev_run_phrase,
        recent_form_phrase, surface_jp,
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
