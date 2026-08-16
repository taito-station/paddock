use core::marker::PhantomData;

use crate::error::{Error, Result};

/// A single payout odds figure, e.g. `3.5` for a win bet.
///
/// JRA quotes odds with one decimal place for win/place/quinella and integer
/// figures for the larger trifecta pools; all of them are `>= 1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OddsValue {
    value: f64,
    _hide_default_constructor: PhantomData<()>,
}

impl OddsValue {
    pub fn value(&self) -> f64 {
        self.value
    }
}

/// Sentinel figures netkeiba publishes for combinations that are not on sale
/// (未発売・該当なし). They are placeholders, not payout odds (#621).
///
/// 実測（2026-08 時点の DB）: ワイド `9999.9` / 馬連・馬単・三連複 `99999.9` / 三連単 `999999.9`。
/// 三連複の `99999.9` は 1 点で `EV = hit_prob * odds` を 3 桁に押し上げ、ポートフォリオの
/// 参考 ROI を 600% 超にしていた。
///
/// **上限方式を採らない**のは、三連単に `111971.9` / `200886.6` のような正当な高配当が実在するため
/// （安易な上限は大穴を殺す）。番兵は固定値なので特定値の除外の方が誤爆しない。
///
/// リストの正本は `netkeiba_sentinels.txt`。Python の分析スクリプト
/// （`scripts/predict-check/odds_guard.py`）も同じファイルを読む。
const NETKEIBA_SENTINELS: [f64; 3] = [9999.9, 99999.9, 999999.9];

/// 番兵値との比較許容差。DB の `double precision` を往復しても取りこぼさないよう幅を持たせる。
const SENTINEL_EPSILON: f64 = 1e-6;

fn is_netkeiba_sentinel(value: f64) -> bool {
    NETKEIBA_SENTINELS
        .iter()
        .any(|sentinel| (value - sentinel).abs() < SENTINEL_EPSILON)
}

impl TryFrom<f64> for OddsValue {
    type Error = Error;
    /// Reject non-odds figures: out-of-range values and netkeiba's unpriced sentinels.
    ///
    /// この 1 か所が値域判定の単一ソース。`save_race_odds::classify_row` と
    /// `find_race_odds::parse_odds_value` が委譲しているので、保存・読み出しの双方に同時に効く
    /// ——**既に DB に入っている番兵行も読み出し時に無害化される**（#621）。
    fn try_from(value: f64) -> Result<Self> {
        if !value.is_finite() || value < 1.0 {
            return Err(Error::OutOfRange(format!(
                "OddsValue must be a finite value >= 1.0, got {value}"
            )));
        }
        if is_netkeiba_sentinel(value) {
            return Err(Error::UnpricedSentinel(format!(
                "netkeiba marks this combination as not on sale ({value} is a placeholder)"
            )));
        }
        Ok(Self {
            value,
            _hide_default_constructor: PhantomData,
        })
    }
}

/// Place (複勝) odds are published as a `low`..`high` band rather than a single
/// figure, because the payout depends on how many horses finish in the money.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaceOdds {
    pub low: OddsValue,
    pub high: OddsValue,
}

impl TryFrom<(OddsValue, OddsValue)> for PlaceOdds {
    type Error = Error;
    /// Build a place-odds band, rejecting an inverted `low > high` range.
    fn try_from((low, high): (OddsValue, OddsValue)) -> Result<Self> {
        if low.value() > high.value() {
            return Err(Error::OutOfRange(format!(
                "PlaceOdds low ({}) must be <= high ({})",
                low.value(),
                high.value()
            )));
        }
        Ok(Self { low, high })
    }
}

#[cfg(test)]
mod tests {
    use super::{NETKEIBA_SENTINELS, OddsValue};

    /// 番兵リストの正本。Python（`scripts/predict-check/odds_guard.py`）も同じファイルを読む。
    /// `include_str!` なのでファイルが消えれば**テストビルド**が通らない（本体ビルドは通る）。
    const SENTINEL_GOLDEN: &str = include_str!("netkeiba_sentinels.txt");

    #[test]
    fn sentinel_list_matches_the_shared_golden() {
        // 片方だけ番兵を足すと必ずここか Python 側が落ちる（#587 の見出し golden と同じ結び方）。
        let golden: Vec<f64> = SENTINEL_GOLDEN
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().parse().expect("golden は 1 行 1 値の f64"))
            .collect();
        assert_eq!(golden, NETKEIBA_SENTINELS.to_vec());
    }

    #[test]
    fn rejects_netkeiba_unpriced_sentinels() {
        // これらは「まだ売れていない」を表す番兵で、払戻倍率ではない（#621）。
        for sentinel in NETKEIBA_SENTINELS {
            let err = OddsValue::try_from(sentinel)
                .expect_err("番兵値をオッズとして受け入れてはいけない");
            assert!(format!("{err}").contains("sentinel"), "{err}");
        }
    }

    #[test]
    fn accepts_legitimate_long_shot_odds() {
        // 三連単には実在する高配当。上限方式を採らない理由そのものなので、通ることを固定する。
        for odds in [111971.9, 200886.6, 99998.9, 100000.0, 999999.8] {
            assert!(
                OddsValue::try_from(odds).is_ok(),
                "正当な高配当を落としてはいけない: {odds}"
            );
        }
    }

    #[test]
    fn rejects_out_of_range_values() {
        // 従来からの下限側（#114）。番兵の追加で壊していないことを見る。
        for bad in [0.0, 0.9, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(OddsValue::try_from(bad).is_err(), "{bad}");
        }
        // 1.0 ちょうどは合法（元返し）。
        assert_eq!(OddsValue::try_from(1.0).map(|v| v.value()).unwrap(), 1.0);
    }
}
