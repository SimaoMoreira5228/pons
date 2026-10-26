use super::*;

mod classification;
mod diagnostics;
mod metadata;
mod ordering;
mod paths;
mod presence;
mod streams;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../../../../spec/fixtures/login.json")).unwrap()
}

fn level(old: &Document, changed: serde_json::Value, path: &str) -> ChangeLevel {
    let new: Document = serde_json::from_value(changed).unwrap();
    old.diff(&new)
        .changes
        .into_iter()
        .find(|change| change.path == path)
        .unwrap()
        .level
}

fn change_at(old: &Document, new: &Document, path: &str) -> Change {
    old.diff(new)
        .changes
        .into_iter()
        .find(|change| change.path == path)
        .unwrap_or_else(|| panic!("no change reported at {path}"))
}

fn doc(summary: Option<&str>, deprecated: Option<bool>) -> Doc {
    Doc {
        summary: summary.map(str::to_owned),
        description: None,
        deprecated,
        tags: Vec::new(),
    }
}
