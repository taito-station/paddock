use core::marker::PhantomData;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HorseNum {
    value: u32,
    _hide_default_constructor: PhantomData<()>,
}

impl HorseNum {
    pub fn value(&self) -> u32 {
        self.value
    }
}

impl TryFrom<u32> for HorseNum {
    type Error = Error;
    fn try_from(value: u32) -> Result<Self> {
        if !(1..=18).contains(&value) {
            return Err(Error::OutOfRange(format!(
                "HorseNum must be 1..=18, got {value}"
            )));
        }
        Ok(Self {
            value,
            _hide_default_constructor: PhantomData,
        })
    }
}
