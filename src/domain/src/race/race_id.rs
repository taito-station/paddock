use core::marker::PhantomData;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RaceId {
    value: String,
    _hide_default_constructor: PhantomData<()>,
}

impl RaceId {
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl TryFrom<String> for RaceId {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        if value.is_empty() || value.len() > 64 {
            return Err(Error::InvalidLengthRange(format!(
                "RaceId length must be 1..=64, got {}",
                value.len()
            )));
        }
        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == 'R')
        {
            return Err(Error::InvalidFormat(format!(
                "RaceId contains invalid characters: {value}"
            )));
        }
        Ok(Self {
            value,
            _hide_default_constructor: PhantomData,
        })
    }
}

impl TryFrom<&str> for RaceId {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self> {
        Self::try_from(value.to_string())
    }
}

impl core::fmt::Display for RaceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.value)
    }
}
