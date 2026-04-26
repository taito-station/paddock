use crate::error::{Error, Result};

/// How a horse's race ended. JRA result tables call out three abnormal terminations
/// distinct from a finished run; we model the rest as `Finished`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    /// 普通に完走（着順あり）
    Finished,
    /// 競走除外（発走前 / 発走時の障害で除外）
    Scratched,
    /// 出走取消（出馬投票後、発走前に取消）
    Cancelled,
    /// 競走中止（発走後、途中で競走を止めた）
    DidNotFinish,
}

impl ResultStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Scratched => "scratched",
            Self::Cancelled => "cancelled",
            Self::DidNotFinish => "did_not_finish",
        }
    }
}

impl TryFrom<&str> for ResultStatus {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self> {
        match value {
            "finished" => Ok(Self::Finished),
            "scratched" => Ok(Self::Scratched),
            "cancelled" => Ok(Self::Cancelled),
            "did_not_finish" => Ok(Self::DidNotFinish),
            other => Err(Error::InvalidFormat(format!(
                "unknown ResultStatus: {other}"
            ))),
        }
    }
}
