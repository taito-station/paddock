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

impl TryFrom<f64> for OddsValue {
    type Error = Error;
    fn try_from(value: f64) -> Result<Self> {
        if !value.is_finite() || value < 1.0 {
            return Err(Error::OutOfRange(format!(
                "OddsValue must be a finite value >= 1.0, got {value}"
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

impl PlaceOdds {
    /// Build a place-odds band, rejecting an inverted `low > high` range.
    pub fn new(low: OddsValue, high: OddsValue) -> Result<Self> {
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
