use crate::error::{Error, Result};
use crate::horse_result::HorseNum;

/// An unordered pair of distinct horses (馬連 key). Stored sorted ascending so
/// the same pair always maps to one key regardless of input order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pair(HorseNum, HorseNum);

impl Pair {
    pub fn as_tuple(&self) -> (HorseNum, HorseNum) {
        (self.0, self.1)
    }

    /// 永続化キー: 昇順の馬番を `-` で連結する（例 `"1-2"`）。馬連・ワイド共通。
    pub fn to_key(&self) -> String {
        format!("{}-{}", self.0.value(), self.1.value())
    }

    /// `"1-2"` 形式のキーをパースする。順序は [`TryFrom`] が昇順に正規化する。
    pub fn from_key(key: &str) -> Result<Self> {
        let (a, b) = parse_two(key, '-')?;
        Self::try_from((a, b))
    }
}

impl TryFrom<(HorseNum, HorseNum)> for Pair {
    type Error = Error;
    fn try_from((a, b): (HorseNum, HorseNum)) -> Result<Self> {
        if a == b {
            return Err(Error::InvalidFormat(format!(
                "Pair requires distinct horses, got {} twice",
                a.value()
            )));
        }
        let (lo, hi) = if a.value() <= b.value() {
            (a, b)
        } else {
            (b, a)
        };
        Ok(Self(lo, hi))
    }
}

/// An ordered pair of distinct horses (馬単 key): first place, then second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderedPair(HorseNum, HorseNum);

impl OrderedPair {
    pub fn as_tuple(&self) -> (HorseNum, HorseNum) {
        (self.0, self.1)
    }

    /// 永続化キー: 着順どおりの馬番を `>` で連結する（例 `"1>2"`）。
    pub fn to_key(&self) -> String {
        format!("{}>{}", self.0.value(), self.1.value())
    }

    /// `"1>2"` 形式のキーをパースする。`>` の左が 1 着、右が 2 着。
    pub fn from_key(key: &str) -> Result<Self> {
        let (a, b) = parse_two(key, '>')?;
        Self::try_from((a, b))
    }
}

impl TryFrom<(HorseNum, HorseNum)> for OrderedPair {
    type Error = Error;
    fn try_from((first, second): (HorseNum, HorseNum)) -> Result<Self> {
        if first == second {
            return Err(Error::InvalidFormat(format!(
                "OrderedPair requires distinct horses, got {} twice",
                first.value()
            )));
        }
        Ok(Self(first, second))
    }
}

/// An unordered triple of distinct horses (三連複 key). Stored sorted ascending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Triple(HorseNum, HorseNum, HorseNum);

impl Triple {
    pub fn as_tuple(&self) -> (HorseNum, HorseNum, HorseNum) {
        (self.0, self.1, self.2)
    }

    /// 永続化キー: 昇順の馬番を `-` で連結する（例 `"1-2-3"`）。
    pub fn to_key(&self) -> String {
        format!("{}-{}-{}", self.0.value(), self.1.value(), self.2.value())
    }

    /// `"1-2-3"` 形式のキーをパースする。順序は [`TryFrom`] が昇順に正規化する。
    pub fn from_key(key: &str) -> Result<Self> {
        let (a, b, c) = parse_three(key, '-')?;
        Self::try_from((a, b, c))
    }
}

impl TryFrom<(HorseNum, HorseNum, HorseNum)> for Triple {
    type Error = Error;
    fn try_from((a, b, c): (HorseNum, HorseNum, HorseNum)) -> Result<Self> {
        let mut sorted = [a, b, c];
        sorted.sort_by_key(|h| h.value());
        if sorted[0] == sorted[1] || sorted[1] == sorted[2] {
            return Err(Error::InvalidFormat(
                "Triple requires three distinct horses".to_string(),
            ));
        }
        Ok(Self(sorted[0], sorted[1], sorted[2]))
    }
}

/// An ordered triple of distinct horses (三連単 key): 1st, 2nd, 3rd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderedTriple(HorseNum, HorseNum, HorseNum);

impl OrderedTriple {
    pub fn as_tuple(&self) -> (HorseNum, HorseNum, HorseNum) {
        (self.0, self.1, self.2)
    }

    /// 永続化キー: 着順どおりの馬番を `>` で連結する（例 `"1>2>3"`）。
    pub fn to_key(&self) -> String {
        format!("{}>{}>{}", self.0.value(), self.1.value(), self.2.value())
    }

    /// `"1>2>3"` 形式のキーをパースする。`>` の左から 1 着・2 着・3 着。
    pub fn from_key(key: &str) -> Result<Self> {
        let (a, b, c) = parse_three(key, '>')?;
        Self::try_from((a, b, c))
    }
}

impl TryFrom<(HorseNum, HorseNum, HorseNum)> for OrderedTriple {
    type Error = Error;
    fn try_from((first, second, third): (HorseNum, HorseNum, HorseNum)) -> Result<Self> {
        if first == second || first == third || second == third {
            return Err(Error::InvalidFormat(
                "OrderedTriple requires three distinct horses".to_string(),
            ));
        }
        Ok(Self(first, second, third))
    }
}

/// 永続化キーの 1 トークンを馬番としてパースする。`to_key`/`from_key` 専用。
/// `to_key` が吐く正規キー（符号・ゼロ詰めなしの 1..=18）との往復閉路を前提とし、外部からの
/// 手書きキーを想定した厳格パーサではない（`u32::parse` は `"+1"`/`"007"` も受理する。範囲外は
/// 後段の `HorseNum::try_from` が弾く）。
fn parse_num(token: &str) -> Result<HorseNum> {
    let num: u32 = token
        .parse()
        .map_err(|_| Error::InvalidFormat(format!("組合せキーの馬番 '{token}' が不正です")))?;
    HorseNum::try_from(num)
}

/// `sep` 区切りで 2 馬番を取り出す。トークン数が 2 でなければエラー。
fn parse_two(key: &str, sep: char) -> Result<(HorseNum, HorseNum)> {
    let parts: Vec<&str> = key.split(sep).collect();
    if parts.len() != 2 {
        return Err(Error::InvalidFormat(format!(
            "組合せキー '{key}' は '{sep}' 区切りの 2 馬番である必要があります"
        )));
    }
    Ok((parse_num(parts[0])?, parse_num(parts[1])?))
}

/// `sep` 区切りで 3 馬番を取り出す。トークン数が 3 でなければエラー。
fn parse_three(key: &str, sep: char) -> Result<(HorseNum, HorseNum, HorseNum)> {
    let parts: Vec<&str> = key.split(sep).collect();
    if parts.len() != 3 {
        return Err(Error::InvalidFormat(format!(
            "組合せキー '{key}' は '{sep}' 区切りの 3 馬番である必要があります"
        )));
    }
    Ok((parse_num(parts[0])?, parse_num(parts[1])?, parse_num(parts[2])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u32) -> HorseNum {
        HorseNum::try_from(n).unwrap()
    }

    #[test]
    fn pair_key_roundtrip_and_normalizes_order() {
        let p = Pair::try_from((h(2), h(1))).unwrap();
        assert_eq!(p.to_key(), "1-2"); // 昇順正規化
        assert_eq!(Pair::from_key("1-2").unwrap(), p);
        assert_eq!(Pair::from_key("2-1").unwrap(), p); // 入力順は問わない
    }

    #[test]
    fn ordered_pair_key_preserves_order() {
        let op = OrderedPair::try_from((h(3), h(1))).unwrap();
        assert_eq!(op.to_key(), "3>1");
        assert_eq!(OrderedPair::from_key("3>1").unwrap(), op);
        assert_ne!(
            OrderedPair::from_key("1>3").unwrap(),
            op,
            "順序が異なれば別キー"
        );
    }

    #[test]
    fn triple_key_roundtrip_and_normalizes_order() {
        let t = Triple::try_from((h(5), h(1), h(3))).unwrap();
        assert_eq!(t.to_key(), "1-3-5");
        assert_eq!(Triple::from_key("5-1-3").unwrap(), t);
    }

    #[test]
    fn ordered_triple_key_preserves_order() {
        let ot = OrderedTriple::try_from((h(7), h(2), h(9))).unwrap();
        assert_eq!(ot.to_key(), "7>2>9");
        assert_eq!(OrderedTriple::from_key("7>2>9").unwrap(), ot);
    }

    #[test]
    fn from_key_rejects_wrong_arity_and_bad_nums() {
        assert!(Pair::from_key("1").is_err()); // トークン不足
        assert!(Pair::from_key("1-2-3").is_err()); // トークン過多
        assert!(Triple::from_key("1-2").is_err());
        assert!(Pair::from_key("1-x").is_err()); // 非数値
        assert!(Pair::from_key("1-19").is_err()); // 馬番範囲外
        assert!(Pair::from_key("1-1").is_err()); // 同一馬
        assert!(Pair::from_key("1-").is_err()); // 末尾空トークン
        assert!(Pair::from_key("-2").is_err()); // 先頭空トークン
        assert!(Pair::from_key("").is_err()); // 空文字列
        assert!(Triple::from_key("1--3").is_err()); // 中間空トークン
    }

    #[test]
    fn ordered_from_key_rejects_invalid() {
        assert!(OrderedPair::from_key("1").is_err()); // トークン不足
        assert!(OrderedPair::from_key("1>2>3").is_err()); // トークン過多
        assert!(OrderedPair::from_key("1>1").is_err()); // 同一馬
        assert!(OrderedPair::from_key("1>19").is_err()); // 馬番範囲外
        assert!(OrderedPair::from_key("1>").is_err()); // 空トークン
        assert!(OrderedTriple::from_key("1>2").is_err()); // トークン不足
        assert!(OrderedTriple::from_key("1>2>2").is_err()); // 同一馬
        assert!(OrderedTriple::from_key("1>x>3").is_err()); // 非数値
    }
}
