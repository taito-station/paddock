use paddock_domain::RaceCard;
use paddock_use_case::entry_parser::EntryParser;
use paddock_use_case::Result as UcResult;

use crate::extract;
use crate::mutool;

pub struct MutoolEntryParser;

impl EntryParser for MutoolEntryParser {
    fn parse(&self, bytes: &[u8]) -> UcResult<Vec<RaceCard>> {
        let json = mutool::extract_stext_json(bytes).map_err(paddock_use_case::Error::from)?;
        extract::parse_stext(&json).map_err(paddock_use_case::Error::from)
    }
}
