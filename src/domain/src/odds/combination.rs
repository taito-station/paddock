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
        let (lo, hi) = if a.value() <= b.value() { (a, b) } else { (b, a) };
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
