//! 盤の書評（レース書評・馬書評）をルールベースで生成する純関数群（#348）。
//!
//! `HorseExplanation`（条件別 factor・近走フォーム・前走サマリ・#343 枠 lift）と混戦サマリから
//! 日本語の寸評を決定的に組む。人手 `PadPrediction` の短評があれば呼び出し側（board.rs）で上書き
//! する（overlay）。presentation ではなく read ユースケースの一部として use-case 層に置き、CLI 整形
//! `predict-format` とは別系統の「盤向けに簡潔な散文」を返す。

use paddock_domain::{ExplainCategory, HorseExplanation, PrevRunSummary, Surface, Verdict};

use super::board::{BoardHorse, Confusion};

/// レース全体の書評（混戦度＋◎の狙いどころ＋妙味馬）。◎（model_rank==1）が無ければ空文字。
pub(crate) fn race_commentary(confusion: &Confusion, horses: &[BoardHorse]) -> String {
    let Some(axis) = horses.iter().find(|h| h.model_rank == 1) else {
        return String::new();
    };
    let axis_pct = pct(axis.win_prob);
    let mut s = if confusion.is_confused {
        // 相手は top5 より広げない（CLAUDE.md 買い方ルール）ので文言も規約準拠にする。
        format!(
            "混戦。勝率上位が {} 頭拮抗。◎{}（勝率{}%）を軸に相手 top5 まで手広く構えたい。",
            confusion.qualifying_count, axis.horse_name, axis_pct
        )
    } else {
        format!(
            "◎{}（勝率{}%）中心。上位が抜けており軸の信頼度は高い。",
            axis.horse_name, axis_pct
        )
    };
    // 乖離馬（モデル上位×市場人気低）があれば妙味として一言添える。◎（model_rank==1）自身は
    // 除く（contrarian 本命が is_value のとき「◎馬X…妙味は馬X」の二重言及を防ぐ）。
    if let Some(v) = horses.iter().find(|h| h.is_value && h.model_rank != 1) {
        match v.popularity {
            Some(pop) => s.push_str(&format!(" 妙味は{}（{}番人気）。", v.horse_name, pop)),
            None => s.push_str(&format!(" 妙味は{}。", v.horse_name)),
        }
    }
    s
}

/// 馬 1 頭の一行寸評（headline）。得意 factor / 前走上位 / 近走の順で最大 2 点を拾う。
/// 特筆すべき材料が無ければ `None`（盤は数値だけで密度を保つ）。
pub(crate) fn horse_headline(expl: &HorseExplanation) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(f) = expl
        .factors
        .iter()
        .find(|f| f.verdict == Some(Verdict::Strong))
    {
        parts.push(format!("{}が得意", factor_topic(f.category, &f.label)));
    }
    let prev_top = expl
        .prev_run
        .as_ref()
        .and_then(|p| p.finishing_position)
        .filter(|&fin| fin <= 3);
    if let Some(fin) = prev_top {
        parts.push(format!("前走{fin}着"));
    }
    if parts.is_empty() {
        let form = form_word(expl.recent_form).filter(|&w| w != "標準");
        if let Some(w) = form {
            parts.push(format!("近走{w}"));
        }
    }
    (!parts.is_empty()).then(|| format!("{}。", parts.join("・")))
}

/// 馬 1 頭の根拠 bullet（展開パネル用）。factor 各行＋枠 lift＋近走＋前走＋斤量を日本語行にする。
pub(crate) fn horse_detail_lines(expl: &HorseExplanation) -> Vec<String> {
    let mut lines = Vec::new();
    for f in &expl.factors {
        let verdict = match f.verdict {
            Some(Verdict::Strong) => "（得意）",
            Some(Verdict::Weak) => "（苦手）",
            Some(Verdict::Neutral) | None => "",
        };
        lines.push(format!(
            "{}：複勝{}%（{}走）{}",
            factor_topic(f.category, &f.label),
            pct(f.rate.show),
            f.starts,
            verdict
        ));
    }
    // gate_bias_lift は同条件（馬場×頭数）の全枠平均比の複勝 lift。ConditionalGateBias factor 行
    // （絶対複勝率「枠バイアス（ラベル）：…」）と話題が被らないよう「相対有利度」と明示する。
    // 判定・方向・表示すべて「pt に丸めた整数」で統一し、丸めて ±0pt になる微小 lift（例 0.004）は
    // 「+0pt なのに有利」の矛盾を避けるため出さない（判定と表示の丸め方式ズレも防ぐ）。
    if let Some(pt) = expl
        .gate_bias_lift
        .map(|l| (l * 100.0).round() as i64)
        .filter(|&p| p != 0)
    {
        let dir = if pt > 0 { "有利" } else { "不利" };
        lines.push(format!(
            "枠の相対有利度：複勝 {pt:+}pt（同条件の全枠平均比で{dir}）"
        ));
    }
    if let Some(w) = form_word(expl.recent_form) {
        lines.push(format!("近走フォーム：{w}"));
    }
    if let Some(p) = &expl.prev_run {
        lines.push(prev_run_line(p));
    }
    if let (Some(w), Some(mean)) = (expl.weight_carried, expl.field_mean_weight) {
        lines.push(format!("斤量：{:.1}kg（平均比 {:+.1}kg）", w, w - mean));
    }
    lines
}

/// factor カテゴリ＋条件ラベルを話題語に整形（predict-format の factor_phrase と同じ方針・盤向けに簡潔）。
fn factor_topic(category: ExplainCategory, label: &str) -> String {
    match category {
        ExplainCategory::Surface | ExplainCategory::Distance => label.to_string(),
        ExplainCategory::TrackCondition => format!("{label}馬場"),
        ExplainCategory::CourseGate => format!("枠（{}）", gate_label_jp(label)),
        ExplainCategory::ConditionalGateBias => format!("枠バイアス（{}）", gate_label_jp(label)),
        ExplainCategory::Jockey => format!("騎手 {label}"),
        ExplainCategory::Trainer => format!("厩舎 {label}"),
        // 相性 factor（#366(b)・率のみ）。venue/距離は「騎手の◯成績」、コンビは「馬×騎手（騎手名）」、
        // 馬×場は「当場（場名）」。predict-format の factor_phrase と話題語を揃える。
        ExplainCategory::JockeyVenue | ExplainCategory::JockeyDistance => {
            format!("騎手の{label}成績")
        }
        ExplainCategory::JockeyHorseCombo => format!("馬×騎手（{label}）"),
        ExplainCategory::HorseVenue => format!("当場（{label}）"),
    }
}

/// 枠グループの英語ラベル（`"Inner (1-3)"` 等）を日本語に。日本語散文の読みやすさのため
/// （盤書評は読み物）。未知ラベルはそのまま返す。
fn gate_label_jp(label: &str) -> &str {
    match label {
        "Inner (1-3)" => "内 1-3",
        "Middle (4-6)" => "中 4-6",
        "Outer (7-8)" => "外 7-8",
        other => other,
    }
}

fn prev_run_line(p: &PrevRunSummary) -> String {
    let surf = match p.surface {
        Surface::Turf => "芝",
        Surface::Dirt => "ダート",
    };
    let mut s = String::from("前走：");
    match p.finishing_position {
        Some(fin) => s.push_str(&format!("{fin}着")),
        None => s.push_str("着順不明"),
    }
    if let Some(pop) = p.popularity {
        s.push_str(&format!("・{pop}番人気"));
    }
    s.push_str(&format!("（{}{}m", surf, p.distance));
    if let Some(m) = &p.margin {
        s.push_str(&format!("・{m}"));
    }
    s.push('）');
    s
}

/// 近走フォームスコア [0,1] を語に（0.6 以上=好調 / 0.4 以下=不調 / それ以外=標準）。欠落は `None`。
fn form_word(form: Option<f64>) -> Option<&'static str> {
    let f = form?;
    Some(if f >= 0.6 {
        "好調"
    } else if f <= 0.4 {
        "不調"
    } else {
        "標準"
    })
}

/// 率 [0,1] を百分率（四捨五入・整数）に。
fn pct(rate: f64) -> i64 {
    (rate * 100.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use paddock_domain::{FactorExplanation, HorseName, HorseNum, PrevRunSummary, RateTriple};

    fn expl(factors: Vec<FactorExplanation>, form: Option<f64>) -> HorseExplanation {
        HorseExplanation {
            horse_num: HorseNum::try_from(1).unwrap(),
            horse_name: HorseName::try_from("テスト馬".to_string()).unwrap(),
            factors,
            recent_form: form,
            prev_run: None,
            gate_bias_lift: None,
            weight_carried: None,
            field_mean_weight: None,
        }
    }

    fn board_horse(model_rank: u32, win_prob: f64, is_value: bool) -> BoardHorse {
        BoardHorse {
            gate_num: None,
            horse_num: model_rank,
            horse_name: format!("馬{model_rank}"),
            jockey: None,
            win_prob,
            place_prob: 0.0,
            show_prob: 0.0,
            pure_win_prob: win_prob,
            pure_place_prob: 0.0,
            pure_show_prob: 0.0,
            market_implied: None,
            win_odds: None,
            morning_win_odds: None,
            place_odds_low: None,
            place_odds_high: None,
            popularity: is_value.then_some(8),
            model_rank,
            mark: None,
            is_overlay: false,
            is_value,
            finishing_position: None,
            comment: None,
            detail_lines: Vec::new(),
        }
    }

    fn confusion(is_confused: bool, qualifying: u32) -> Confusion {
        Confusion {
            is_confused,
            axis_win_prob: 0.3,
            threshold: 0.21,
            qualifying_count: qualifying,
        }
    }

    #[test]
    fn race_commentary_dominant_axis() {
        let horses = vec![board_horse(1, 0.42, false), board_horse(2, 0.1, false)];
        let s = race_commentary(&confusion(false, 1), &horses);
        assert!(s.contains("◎馬1（勝率42%）中心"), "got: {s}");
        assert!(s.contains("信頼度は高い"));
        assert!(!s.contains("妙味"), "妙味馬なし: {s}");
    }

    #[test]
    fn race_commentary_confused_with_value() {
        let horses = vec![board_horse(1, 0.2, false), board_horse(3, 0.15, true)];
        let s = race_commentary(&confusion(true, 4), &horses);
        assert!(s.contains("混戦。勝率上位が 4 頭拮抗"), "got: {s}");
        assert!(s.contains("妙味は馬3（8番人気）"), "got: {s}");
    }

    #[test]
    fn race_commentary_empty_when_no_axis() {
        assert_eq!(race_commentary(&confusion(false, 0), &[]), "");
    }

    #[test]
    fn horse_headline_picks_strong_factor() {
        let f = FactorExplanation::new(
            ExplainCategory::Surface,
            "芝".to_string(),
            RateTriple {
                win: 0.3,
                place: 0.5,
                show: 0.7,
            },
            20,
        );
        // Surface は verdict 対象。show=0.7 の高率なので Strong。
        assert_eq!(f.verdict, Some(Verdict::Strong));
        let h = horse_headline(&expl(vec![f], None)).unwrap();
        assert_eq!(h, "芝が得意。");
    }

    #[test]
    fn horse_headline_none_when_nothing_notable() {
        // 標準フォーム・factor 無し・前走無し → 特筆材料なし。
        assert_eq!(horse_headline(&expl(vec![], Some(0.5))), None);
    }

    #[test]
    fn horse_detail_lines_render_factor_and_form() {
        let f = FactorExplanation::new(
            ExplainCategory::TrackCondition,
            "重".to_string(),
            RateTriple {
                win: 0.1,
                place: 0.2,
                show: 0.25,
            },
            8,
        );
        let mut e = expl(vec![f], Some(0.7));
        e.gate_bias_lift = Some(0.06);
        let lines = horse_detail_lines(&e);
        assert!(
            lines.iter().any(|l| l.contains("重馬場：複勝25%（8走）")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("枠の相対有利度：複勝 +6pt")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l == "近走フォーム：好調"), "{lines:?}");
    }

    #[test]
    fn affinity_factors_rendered_rate_only() {
        // #366(b) 相性 factor は verdict=None（率のみ）で detail 行に出る。話題語を確認。
        let combo = FactorExplanation::new(
            ExplainCategory::JockeyHorseCombo,
            "武豊".to_string(),
            RateTriple {
                win: 0.2,
                place: 0.35,
                show: 0.5,
            },
            8,
        );
        let jv = FactorExplanation::new(
            ExplainCategory::JockeyVenue,
            "函館".to_string(),
            RateTriple {
                win: 0.1,
                place: 0.2,
                show: 0.28,
            },
            40,
        );
        let hv = FactorExplanation::new(
            ExplainCategory::HorseVenue,
            "函館".to_string(),
            RateTriple {
                win: 0.15,
                place: 0.25,
                show: 0.33,
            },
            6,
        );
        let lines = horse_detail_lines(&expl(vec![combo, jv, hv], None));
        assert!(
            lines.iter().any(|l| l == "馬×騎手（武豊）：複勝50%（8走）"),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "騎手の函館成績：複勝28%（40走）"),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "当場（函館）：複勝33%（6走）"),
            "{lines:?}"
        );
        // 相性 factor は verdict=None ゆえ headline（得意 factor 拾い）には出ない。
        assert_eq!(
            horse_headline(&expl(
                vec![FactorExplanation::new(
                    ExplainCategory::JockeyHorseCombo,
                    "武豊".to_string(),
                    RateTriple {
                        win: 0.4,
                        place: 0.6,
                        show: 0.8,
                    },
                    20,
                )],
                Some(0.5)
            )),
            None
        );
    }

    #[test]
    fn course_gate_label_rendered_in_japanese() {
        // 枠グループの英語ラベルは日本語散文向けに和訳する（読み物としての可読性）。
        let f = FactorExplanation::new(
            ExplainCategory::CourseGate,
            "Inner (1-3)".to_string(),
            RateTriple {
                win: 0.1,
                place: 0.2,
                show: 0.27,
            },
            136,
        );
        let lines = horse_detail_lines(&expl(vec![f], None));
        assert!(
            lines.iter().any(|l| l.contains("枠（内 1-3）：複勝27%")),
            "{lines:?}"
        );
    }

    #[test]
    fn horse_detail_lines_weak_negative_lift_poor_form_and_prev_run() {
        // 反対側の分岐: Weak factor「（苦手）」・負 lift「不利」・不調フォーム・前走行。
        let f = FactorExplanation::new(
            ExplainCategory::TrackCondition,
            "重".to_string(),
            RateTriple {
                win: 0.02,
                place: 0.05,
                show: 0.08,
            },
            30,
        );
        assert_eq!(f.verdict, Some(Verdict::Weak));
        let mut e = expl(vec![f], Some(0.3)); // 不調
        e.gate_bias_lift = Some(-0.04); // 不利
        e.prev_run = Some(PrevRunSummary {
            finishing_position: Some(8),
            popularity: Some(5),
            margin: Some("2馬身".to_string()),
            surface: Surface::Dirt,
            distance: 1600,
        });
        let lines = horse_detail_lines(&e);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("重馬場：複勝8%（30走）（苦手）")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("枠の相対有利度：複勝 -4pt") && l.contains("不利")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l == "近走フォーム：不調"), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l == "前走：8着・5番人気（ダート1600m・2馬身）"),
            "{lines:?}"
        );
    }

    #[test]
    fn horse_detail_lines_hides_negligible_lift() {
        // 丸めて ±0pt になる微小 lift は矛盾表示を避けるため出さない。
        let mut e = expl(vec![], None);
        e.gate_bias_lift = Some(0.004);
        assert!(
            !horse_detail_lines(&e)
                .iter()
                .any(|l| l.contains("枠の相対有利度")),
            "微小 lift は非表示"
        );
    }

    #[test]
    fn race_commentary_value_without_popularity() {
        // 妙味馬の popularity が None のとき「妙味は馬X。」（人気表記なし）。
        let mut v = board_horse(2, 0.15, true);
        v.popularity = None;
        let horses = vec![board_horse(1, 0.4, false), v];
        let s = race_commentary(&confusion(false, 1), &horses);
        assert!(s.contains("妙味は馬2。"), "got: {s}");
        assert!(!s.contains("番人気"), "人気表記は出ない: {s}");
    }
}
