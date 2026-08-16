use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid format: {0}")]
    InvalidFormat(String),
    #[error("invalid length range: {0}")]
    InvalidLengthRange(String),
    #[error("out of range: {0}")]
    OutOfRange(String),
    /// netkeiba が未発売の組み合わせに入れる番兵値（#621）。
    ///
    /// `OutOfRange` と分けているのは**ログレベルを出し分けるため**。番兵は異常ではなく
    /// 「まだ売れていない」という正常な状態で、1 レースに数百件出ることもある。
    /// これを warn で出すと本来の値域違反（旧ダンプ由来の残骸など）が埋もれる。
    #[error("unpriced sentinel: {0}")]
    UnpricedSentinel(String),
}

pub type Result<A> = std::result::Result<A, Error>;
