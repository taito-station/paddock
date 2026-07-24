use paddock_domain::{
    ExplainCategory, FactorExplanation, HorseEntry, HorseExplanation, PrevRunSummary, RateTriple,
    RecentRun,
};

use crate::repository::{ConditionalGateStatsRow, gate_field_band_label, gate_track_cond2_label};

use super::features::{RaceContext, SharedFactorStats};

/// 取得済みの stats 行・前走から各馬の予想根拠 [`HorseExplanation`] を組み立てる純粋変換（#274）。
/// `build_factors` と同じ [`SharedFactorStats`]（同一ラベルの集計レート）を読み、score への合成ではなく
/// 「人が読める条件別成績」へ写像する。共有構造体を両者で読むため「根拠の数値＝score の入力」が構造的に
/// 一致し、かつてのラベル選択・集計の手動同期（factor 追加時に両方更新）は不要になった（#409）。
/// `prev_run` は最新走（runs の index 0）。conditional_gate（枠バイアス・提示専用）と prev_run は根拠固有
/// のため本関数が自前で扱う（score には投入しない）。
pub(crate) fn build_explanation(
    shared: &SharedFactorStats,
    entry: &HorseEntry,
    conditional_gate: Option<&ConditionalGateStatsRow>,
    race: &RaceContext,
    recent_form: Option<f64>,
    prev_run: Option<&RecentRun>,
) -> HorseExplanation {
    let mut factors: Vec<FactorExplanation> = Vec::new();
    // 馬の条件別成績（芝ダ・距離帯）。共有の集計 FactorStat をそのまま使う（None は母数除外の欠落扱い）。
    if let Some(fs) = shared.horse_surface {
        factors.push(FactorExplanation::new(
            ExplainCategory::Surface,
            shared.surf_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    if let Some(fs) = shared.horse_distance {
        factors.push(FactorExplanation::new(
            ExplainCategory::Distance,
            shared.dist_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    // 馬場状態は当日値が確定しているレースのみ（未確定なら shared.horse_track_condition が None）。
    if let Some(tc) = race.track_condition
        && let Some(fs) = shared.horse_track_condition
    {
        factors.push(FactorExplanation::new(
            ExplainCategory::TrackCondition,
            tc.as_str().to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    // コース×枠（場×距離×馬場の枠順別）。全馬共通の course から（shared.course_gate）。
    if let Some(fs) = shared.course_gate {
        factors.push(FactorExplanation::new(
            ExplainCategory::CourseGate,
            shared.gate_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    // 条件依存枠バイアス（馬場×頭数×枠, #343・提示専用・スコア非投入）。馬場が確定し、集計セルに実績が
    // あるときだけ提示する。lift（同条件全枠平均との差）を HorseExplanation に載せ、市場差分フラグ判定に使う。
    let mut gate_bias_lift: Option<f64> = None;
    if let Some(cg) = conditional_gate
        && let Some(tc) = race.track_condition
    {
        let track_label = gate_track_cond2_label(tc.as_str());
        let field_label = gate_field_band_label(race.field_size as u32);
        if let Some(cell) = cg.cell(track_label, field_label, shared.gate_label)
            && cell.stat.starts > 0
        {
            factors.push(FactorExplanation::new(
                ExplainCategory::ConditionalGateBias,
                format!(
                    "{} / {} / {}",
                    gate_bias_gate_jp(entry.gate_num.value()),
                    track_label,
                    field_label
                ),
                RateTriple {
                    win: cell.stat.win_rate(),
                    place: cell.stat.place_rate(),
                    show: cell.stat.show_rate(),
                },
                cell.stat.starts,
            ));
            // 枠効果 lift = セル複勝率 − 同条件(馬場×頭数)の全枠平均複勝率。
            if let Some(base) = cg.condition_show_rate(track_label, field_label) {
                gate_bias_lift = Some(cell.stat.show_rate() - base);
            }
        }
    }
    // 騎手・調教師（芝ダ別）。未登録・実績なしは項を立てない（shared が None）。ラベルは entry の名前を使う。
    if let Some(fs) = shared.jockey_surface {
        let label = entry
            .jockey
            .as_ref()
            .map(|n| n.value().to_string())
            .unwrap_or_default();
        factors.push(FactorExplanation::new(
            ExplainCategory::Jockey,
            label,
            fs.rate,
            fs.starts,
        ));
    }
    if let Some(fs) = shared.trainer_surface {
        let label = entry
            .trainer
            .as_ref()
            .map(|n| n.value().to_string())
            .unwrap_or_default();
        factors.push(FactorExplanation::new(
            ExplainCategory::Trainer,
            label,
            fs.rate,
            fs.starts,
        ));
    }
    // #366(b) 相性 factor を書評の根拠に載せる（描画層のみ・本番 weight は不変）。build_factors と同じ
    // 照合キー・`stat_to_triple_opt` を使い「実績がある時だけ書く」欠落扱い（母数 0・欠落は項を立てない）。
    // coverage が薄い combo/horse_venue も starts>0 の時だけ出る。率のみ提示（verdict=None）。
    if let Some(fs) = shared.jockey_venue {
        factors.push(FactorExplanation::new(
            ExplainCategory::JockeyVenue,
            shared.venue_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    if let Some(fs) = shared.jockey_distance {
        factors.push(FactorExplanation::new(
            ExplainCategory::JockeyDistance,
            shared.dist_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    // 馬×騎手コンビ: この馬の騎手別成績（horse.by_jockey）を現騎手名で引く（shared と同源）。ラベルは現騎手名。
    if let Some(jn) = entry.jockey.as_ref()
        && let Some(fs) = shared.jockey_horse_combo
    {
        factors.push(FactorExplanation::new(
            ExplainCategory::JockeyHorseCombo,
            jn.value().to_string(),
            fs.rate,
            fs.starts,
        ));
    }
    if let Some(fs) = shared.horse_venue {
        factors.push(FactorExplanation::new(
            ExplainCategory::HorseVenue,
            shared.venue_label.to_string(),
            fs.rate,
            fs.starts,
        ));
    }

    let prev_run = prev_run.map(|r| PrevRunSummary {
        finishing_position: r.result.finishing_position.map(|p| p.value()),
        popularity: r.result.popularity,
        margin: r.result.margin.clone(),
        surface: r.surface,
        distance: r.distance,
    });

    HorseExplanation {
        horse_num: entry.horse_num,
        horse_name: entry.horse_name.clone(),
        factors,
        recent_form,
        prev_run,
        gate_bias_lift,
        weight_carried: entry.weight_carried,
        field_mean_weight: race.mean_weight,
    }
}

/// 枠番（1..=8, `GateNum` 検証済み）を「内枠/中枠/外枠」に写す（#343 提示ラベル用）。英字ラベル文字列に
/// 依存せず枠番から直接引くことで、集計側 `GATE_GROUPS` ラベルとの二重定義を 1 つ減らす（レビュー指摘）。
fn gate_bias_gate_jp(gate_num: u32) -> &'static str {
    match gate_num {
        1..=3 => "内枠",
        4..=6 => "中枠",
        // GateNum は 1..=8 検証済みなので _ は 7-8（外枠）のみ（`gate_group_label` と同じ区分）。
        _ => "外枠",
    }
}
