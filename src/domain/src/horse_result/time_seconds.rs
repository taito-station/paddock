use core::marker::PhantomData;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSeconds {
    value: f64,
    _hide_default_constructor: PhantomData<()>,
}

impl TimeSeconds {
    pub fn value(&self) -> f64 {
        self.value
    }
}

impl TryFrom<f64> for TimeSeconds {
    type Error = Error;
    fn try_from(value: f64) -> Result<Self> {
        if !value.is_finite() || !(0.0..=600.0).contains(&value) {
            return Err(Error::OutOfRange(format!(
                "TimeSeconds must be a finite value in 0.0..=600.0, got {value}"
            )));
        }
        Ok(Self {
            value,
            _hide_default_constructor: PhantomData,
        })
    }
}

impl TimeSeconds {
    /// Parse "M:SS.f" (e.g. "1:23.4" or "1：11．6") into total seconds.
    pub fn try_from_mss_str(raw: &str) -> Result<Self> {
        let normalized: String = raw
            .chars()
            .map(|c| match c {
                '：' => ':',
                '．' => '.',
                _ => c,
            })
            .filter(|c| !c.is_whitespace())
            .collect();
        let (m_str, s_str) = normalized.split_once(':').ok_or_else(|| {
            Error::InvalidFormat(format!("time '{raw}' does not contain ':' separator"))
        })?;
        let minutes: f64 = m_str
            .parse()
            .map_err(|e| Error::InvalidFormat(format!("invalid minutes in '{raw}': {e}")))?;
        let seconds: f64 = s_str
            .parse()
            .map_err(|e| Error::InvalidFormat(format!("invalid seconds in '{raw}': {e}")))?;
        Self::try_from(minutes * 60.0 + seconds)
    }
}
