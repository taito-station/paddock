//! リーセンシー重み付け（recency, #75 Phase B）。日付付き成績系列に時間減衰を掛けて集計する。

use chrono::NaiveDate;

use super::model::{DatedCounts, FactorStat, RateTriple};

/// 日付付き成績系列に時間減衰 `w = 0.5^((as_of − date)/half_life)` を掛け、時間重み付きレート
/// （`Σ w·wins / Σ w·starts` 等）と総出走数を `FactorStat` で返す（#75 Phase B）。直近走ほど
/// 重みが大きく、半減期 `half_life_days` 日で寄与が半分になる。`as_of` 以降の日付はリーク防止の
/// ため無視する（呼び出し側が as_of で絞るが二重防御）。有効な重み付き出走が無ければ `None`。
///
/// `FactorStat.starts` は時間重みを掛けない素の総出走数を返す。recency と shrinkage を併用すると
/// 縮約はこの素の starts を信頼度 k に使う（＝減衰で薄れた古い実績も母数に満額カウント）。この
/// 非対称は割り切りで、併用経路は backtest（CLI 両指定）でのみ到達し本番 predict では走らない
/// （`production()` は recency 無効）。recency を将来採用する際は減衰後の実効標本数での縮約を
/// 再検討する（ADR 0016）。
pub fn apply_recency_weight(
    runs: &[DatedCounts],
    as_of: NaiveDate,
    half_life_days: f64,
) -> Option<FactorStat> {
    // 呼び出し側（CLI `--recency-half-life`）が有限の正数を保証する。万一 0・負・非有限が来ても
    // `0.5^(±inf)` 等で全重み 0 → None に倒れ NaN は出さないが、契約違反は debug ビルドで検出する。
    debug_assert!(
        half_life_days.is_finite() && half_life_days > 0.0,
        "half_life_days must be finite and positive, got {half_life_days}"
    );
    let mut w_starts = 0.0;
    let mut w_wins = 0.0;
    let mut w_places = 0.0;
    let mut w_shows = 0.0;
    let mut total_starts: u32 = 0;
    for r in runs {
        let days_ago = (as_of - r.date).num_days();
        // as_of 当日・以降はリークになるため寄与させない（< as_of のみ）。
        if days_ago <= 0 {
            continue;
        }
        let w = 0.5_f64.powf(days_ago as f64 / half_life_days);
        w_starts += w * r.starts as f64;
        w_wins += w * r.wins as f64;
        w_places += w * r.places as f64;
        w_shows += w * r.shows as f64;
        // 実データでは 1 頭の生涯出走数は高々数十だが、契約外の入力でも安全側に倒す。
        total_starts = total_starts.saturating_add(r.starts);
    }
    if w_starts <= 0.0 {
        return None;
    }
    Some(FactorStat {
        rate: RateTriple {
            win: w_wins / w_starts,
            place: w_places / w_starts,
            show: w_shows / w_starts,
        },
        starts: total_starts,
    })
}
