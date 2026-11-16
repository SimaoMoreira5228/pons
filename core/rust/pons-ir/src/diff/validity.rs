use crate::{Document, ValidationError};

use super::{ChangeLevel, Diff, add};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validity {
    pub previous: Vec<ValidationError>,

    pub new: Vec<ValidationError>,
}

impl Validity {
    pub fn previous_is_valid(&self) -> bool {
        self.previous.is_empty()
    }

    pub fn new_is_valid(&self) -> bool {
        self.new.is_empty()
    }
}

pub(super) fn diff_validity(diff: &mut Diff, old: &Document, new: &Document) {
    let previous = old.validate().err().unwrap_or_default();
    let new = new.validate().err().unwrap_or_default();
    match (previous.is_empty(), new.is_empty()) {
        (true, true) => {}
        (false, true) => add(
            diff,
            ChangeLevel::Info,
            "document",
            format!(
                "the previous document was invalid: {}",
                validation_message(&previous)
            ),
        ),
        (true, false) => add(
            diff,
            ChangeLevel::Breaking,
            "document",
            format!("the new document is invalid: {}", validation_message(&new)),
        ),
        (false, false) => add(
            diff,
            ChangeLevel::Warning,
            "document",
            format!(
                "both documents are invalid: the previous document: {}; the new document: {}",
                validation_message(&previous),
                validation_message(&new)
            ),
        ),
    }
    diff.validity = Validity { previous, new };
}

fn validation_message(errors: &[ValidationError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
