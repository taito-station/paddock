use core::marker::PhantomData;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GateNum {
    value: u32,
    _hide_default_constructor: PhantomData<()>,
}

impl GateNum {
    pub fn value(&self) -> u32 {
        self.value
    }
}

impl TryFrom<u32> for GateNum {
    type Error = Error;
    fn try_from(value: u32) -> Result<Self> {
        if !(1..=8).contains(&value) {
            return Err(Error::OutOfRange(format!(
                "GateNum must be 1..=8, got {value}"
            )));
        }
        Ok(Self {
            value,
            _hide_default_constructor: PhantomData,
        })
    }
}
