use core::marker::PhantomData;

use crate::error::{Error, Result};
use crate::odds::BetType;

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
/// **番兵は券種別**（#630/#634）。netkeiba の番兵は券種ごとに固定値で、実測（2026-08 時点の DB）:
/// ワイド `9999.9` / 馬連・馬単・三連複 `99999.9` / 三連単 `999999.9`。**単勝・複勝に番兵は無い**
/// （DB 実測で該当 0 行・#634）。券種を見ずにフラットに弾くと、三連複・三連単の**正当な**
/// `9999.9`（9000〜11000 帯に trio 6,244 行・trifecta 56,230 行が実在=2026-08-18 実測）まで巻き添えで落とす。
///
/// **上限方式を採らない**のは、三連単に `111971.9` / `200886.6` のような正当な高配当が実在するため
/// （安易な上限は大穴を殺す）。番兵は券種ごとの固定値なので特定値の除外の方が誤爆しない。
///
/// **リストの正本は `netkeiba_sentinels.txt`**（TAB 区切り `券種<TAB>値` の 2 列。番兵を持たない
/// 券種は行そのものを置かない）。Python の分析スクリプト（`scripts/predict-check/odds_guard.py`）が
/// **import 時に**同じファイルを読むため、これはテスト専用資産ではなく**本番依存**——`testdata/` に
/// 移さないこと。こちらは下の `sentinel_list_matches_the_shared_golden` が `include_str!` で
/// 突き合わせる。
///
/// 値を足すときは 3 か所を同じ PR で更新する: 正本ファイル / この定数 /
/// `scripts/predict-check/test_odds_guard.py` の期待 dict。1 つでも忘れれば
/// Rust か Python のテストが落ちる。運用上の位置づけは
/// `docs/specifications/netkeiba-datasource.md` の番兵の節が正。
const NETKEIBA_SENTINELS: [(BetType, f64); 5] = [
    (BetType::Wide, 9999.9),
    (BetType::Quinella, 99999.9),
    (BetType::Exacta, 99999.9),
    (BetType::Trio, 99999.9),
    (BetType::Trifecta, 999999.9),
];

/// 番兵値との比較許容差。DB の `double precision` を往復しても取りこぼさないよう幅を持たせる。
const SENTINEL_EPSILON: f64 = 1e-6;

fn is_netkeiba_sentinel(bet_type: BetType, value: f64) -> bool {
    NETKEIBA_SENTINELS
        .iter()
        .any(|(bt, sentinel)| *bt == bet_type && (value - sentinel).abs() < SENTINEL_EPSILON)
}

impl TryFrom<(BetType, f64)> for OddsValue {
    type Error = Error;
    /// Reject non-odds figures: out-of-range values and netkeiba's unpriced sentinels.
    ///
    /// この 1 か所が値域判定の単一ソース。`save_race_odds::classify_row` と
    /// `find_race_odds::parse_odds_value` が委譲しているので、保存・読み出しの双方に同時に効く
    /// ——**既に DB に入っている番兵行も読み出し時に無害化される**（#621）。
    ///
    /// **券種を必須入力にする**（#630）。番兵は券種別の固定値なので、券種なしでは
    /// 「三連複の正当な `9999.9`」と「ワイドの番兵 `9999.9`」を区別できない。`TryFrom<f64>` を
    /// 残さないのは意図的——券種を渡し忘れた新しい呼び出し口を**コンパイルエラー**にすることが、
    /// #621 の失敗様式（ガードを通らない経路が静かに増える）への唯一の構造的な防御。
    fn try_from((bet_type, value): (BetType, f64)) -> Result<Self> {
        if !value.is_finite() || value < 1.0 {
            return Err(Error::OutOfRange(format!(
                "OddsValue must be a finite value >= 1.0, got {value}"
            )));
        }
        if is_netkeiba_sentinel(bet_type, value) {
            return Err(Error::UnpricedSentinel(format!(
                "netkeiba marks this {bet_type} combination as not on sale ({value} is a placeholder)"
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
    use super::{BetType, NETKEIBA_SENTINELS, OddsValue};

    /// 番兵リストの正本。Python（`scripts/predict-check/odds_guard.py`）も同じファイルを読む。
    /// `include_str!` なのでファイルが消えれば**テストビルド**が通らない（本体ビルドは通る）。
    const SENTINEL_GOLDEN: &str = include_str!("netkeiba_sentinels.txt");

    #[test]
    fn sentinel_list_matches_the_shared_golden() {
        // 片方だけ番兵を足すと必ずここか Python 側が落ちる（#587 の見出し golden と同じ結び方）。
        // 書式は TAB 区切り `券種<TAB>値` の 2 列（#630）。順序込みで突き合わせる。
        let golden: Vec<(BetType, f64)> = SENTINEL_GOLDEN
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (label, value) = l
                    .trim()
                    .split_once('\t')
                    .expect("golden は `券種<TAB>値` の 2 列");
                (
                    // label も trim する——Python 側は label/value を個別に strip しており、
                    // `wide␣<TAB>` のような行で両言語の受理が非対称にならないよう揃える。
                    BetType::try_from(label.trim())
                        .expect("golden の券種ラベルは snake_case の既知の値"),
                    value.trim().parse().expect("golden の値は f64"),
                )
            })
            .collect();
        assert_eq!(golden, NETKEIBA_SENTINELS.to_vec());
    }

    #[test]
    fn win_and_place_have_no_sentinels() {
        // #634: 単勝・複勝に番兵は無い（DB 実測 0 行）。行が「無い」ことは golden 突合だけでは
        // 固定されない（定数と一緒に足せば通ってしまう）ので、独立に assert する。
        assert!(
            NETKEIBA_SENTINELS
                .iter()
                .all(|(bt, _)| !matches!(bt, BetType::Win | BetType::Place)),
            "win/place に番兵を足すときは #634 の実測を覆す根拠を PR に示すこと"
        );
        // 他券種の番兵値でも win/place では正当なオッズとして通る。
        for v in [9999.9, 99999.9, 999999.9] {
            assert!(OddsValue::try_from((BetType::Win, v)).is_ok(), "{v}");
            assert!(OddsValue::try_from((BetType::Place, v)).is_ok(), "{v}");
        }
    }

    #[test]
    fn sentinel_rejection_is_scoped_to_the_bet_type() {
        // 7 券種 × 3 番兵値の受理/拒否マトリクス（#630）。期待値は定数から導出せず**手で書く**
        // ——定数の書き換え事故がこの表とぶつかって赤くなるようにする。
        // 核は (Trio, 9999.9) == Ok（正当な三連複オッズ）と (Wide, 9999.9) == Err（ワイドの番兵）。
        use BetType::*;
        let cases: [(BetType, f64, bool); 21] = [
            (Win, 9999.9, false),
            (Win, 99999.9, false),
            (Win, 999999.9, false),
            (Place, 9999.9, false),
            (Place, 99999.9, false),
            (Place, 999999.9, false),
            (Quinella, 9999.9, false),
            (Quinella, 99999.9, true),
            (Quinella, 999999.9, false),
            (Wide, 9999.9, true),
            (Wide, 99999.9, false),
            (Wide, 999999.9, false),
            (Exacta, 9999.9, false),
            (Exacta, 99999.9, true),
            (Exacta, 999999.9, false),
            (Trio, 9999.9, false),
            (Trio, 99999.9, true),
            (Trio, 999999.9, false),
            (Trifecta, 9999.9, false),
            (Trifecta, 99999.9, false),
            (Trifecta, 999999.9, true),
        ];
        for (bet_type, value, expect_rejected) in cases {
            let got = OddsValue::try_from((bet_type, value));
            if expect_rejected {
                let err =
                    got.expect_err(&format!("{bet_type} の番兵 {value} を受け入れてはいけない"));
                assert!(
                    format!("{err}").contains("sentinel"),
                    "{bet_type} {value}: {err}"
                );
            } else {
                assert!(
                    got.is_ok(),
                    "{bet_type} の正当なオッズ {value} を落としてはいけない"
                );
            }
        }
    }

    #[test]
    fn accepts_legitimate_long_shot_odds() {
        // 三連単には実在する高配当。上限方式を採らない理由そのものなので、通ることを固定する。
        for odds in [111971.9, 200886.6, 99998.9, 100000.0, 999999.8] {
            assert!(
                OddsValue::try_from((BetType::Trifecta, odds)).is_ok(),
                "正当な高配当を落としてはいけない: {odds}"
            );
        }
    }

    #[test]
    fn rejects_out_of_range_values() {
        // 従来からの下限側（#114）。番兵の券種別化で壊していないこと・券種に依らないことを見る。
        for bad in [0.0, 0.9, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(OddsValue::try_from((BetType::Win, bad)).is_err(), "{bad}");
            assert!(OddsValue::try_from((BetType::Trio, bad)).is_err(), "{bad}");
        }
        // 1.0 ちょうどは合法（元返し）。
        assert_eq!(
            OddsValue::try_from((BetType::Win, 1.0))
                .map(|v| v.value())
                .unwrap(),
            1.0
        );
    }
}
