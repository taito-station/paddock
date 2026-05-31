use paddock_domain::RaceCard;

use crate::error::Result;

pub trait EntryParser: Send + Sync {
    fn parse(&self, bytes: &[u8]) -> Result<Vec<RaceCard>>;
}
