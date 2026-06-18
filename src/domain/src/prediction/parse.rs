//! 前走着差文字列のパース（#76）。複数出典（JRA PDF / netkeiba）の表記を馬身へ正規化する。

use super::weights::MARGIN_CAP_LENGTHS;

/// 前走着差文字列を馬身（length）に変換する（#76）。複数出典の表記を吸収する:
/// キーワード（`ハナ`/`アタマ`/`クビ`/`大差`/`同着`）、分数（`3/4`・整数+分数 `1.1/4`）、
/// 小数・整数（`0.6`/`2`）。解釈できない・空文字・負値は `None`（signal を母数から除外）。
pub(crate) fn parse_margin_lengths(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // キーワード表記。PDF パーサはハナ/アタマ/クビのみ、netkeiba は大差・同着も返す。
    // 馬身換算は JRA の慣行値（ハナ<アタマ<クビ）に倣う近似。
    if t.contains("同着") {
        return Some(0.0);
    }
    if t.contains("大差") {
        // クランプ点と同じ定数を返し、「大差」を必ず競争力差の最大（margin_form で mag=1.0）に揃える。
        // 片方だけ調整すると意図がずれるため二役であることを明示。
        return Some(MARGIN_CAP_LENGTHS);
    }
    if t.contains("ハナ") {
        return Some(0.05);
    }
    if t.contains("アタマ") {
        return Some(0.10);
    }
    if t.contains("クビ") {
        return Some(0.25);
    }
    // 分数表記。`/` を含むとき、`.` があれば整数部+分数部（`1.1/4` = 1 + 1/4）、無ければ分数のみ。
    if t.contains('/') {
        if let Some(dot) = t.find('.') {
            let whole: f64 = t[..dot].trim().parse().ok()?;
            let frac = parse_fraction(&t[dot + 1..])?;
            return Some(whole + frac);
        }
        return parse_fraction(t);
    }
    // 小数・整数（`0.6` / `2` / `1.0`）。負値・非有限は弾く。
    t.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

/// `A/B` 形式の分数文字列を解釈する。パース不能・分母 0・負値・非有限は `None`
/// （小数経路と同じく着差は非負のみ受ける）。
fn parse_fraction(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let num: f64 = num.trim().parse().ok()?;
    let den: f64 = den.trim().parse().ok()?;
    if den == 0.0 {
        return None;
    }
    let v = num / den;
    (v.is_finite() && v >= 0.0).then_some(v)
}
