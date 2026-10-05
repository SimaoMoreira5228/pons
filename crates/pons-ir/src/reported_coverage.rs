use std::collections::BTreeSet;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::{
    CoverageLevel, Document, Operation, canonical_coverage, operation_path, strict_violations,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub exact: BTreeSet<String>,
    pub opaque: BTreeSet<String>,
    pub lossy: BTreeSet<String>,
    pub unsupported: BTreeSet<String>,
}

impl Coverage {
    pub fn levels(&self) -> impl Iterator<Item = (String, CoverageLevel)> {
        [
            (&self.exact, CoverageLevel::Exact),
            (&self.opaque, CoverageLevel::Opaque),
            (&self.lossy, CoverageLevel::Lossy),
            (&self.unsupported, CoverageLevel::Unsupported),
        ]
        .into_iter()
        .flat_map(|(subjects, level)| subjects.iter().map(move |subject| (subject.clone(), level)))
    }
}

impl<'de> Deserialize<'de> for Coverage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(default)]
            exact: Vec<String>,
            #[serde(default)]
            opaque: Vec<String>,
            #[serde(default)]
            lossy: Vec<String>,
            #[serde(default)]
            unsupported: Vec<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Coverage {
            exact: coverage_set("exact", raw.exact)?,
            opaque: coverage_set("opaque", raw.opaque)?,
            lossy: coverage_set("lossy", raw.lossy)?,
            unsupported: coverage_set("unsupported", raw.unsupported)?,
        })
    }
}

fn coverage_set<E: de::Error>(level: &str, values: Vec<String>) -> Result<BTreeSet<String>, E> {
    let mut set = BTreeSet::new();
    for value in values {
        if !set.insert(value.clone()) {
            return Err(E::custom(format!(
                "emitter coverage lists {value} more than once in {level} coverage"
            )));
        }
    }
    Ok(set)
}

pub fn strict_violation_message(coverage: &Coverage) -> Option<String> {
    let violations: Vec<String> = strict_violations(coverage.levels(), true)
        .into_iter()
        .map(|(level, subjects)| {
            format!("{level} coverage in strict mode: {}", subjects.join(", "))
        })
        .collect();
    (!violations.is_empty()).then(|| violations.join("; "))
}

pub fn coverage_for_document(
    document: &Document,
    lossy_wide_integers: bool,
) -> Result<Coverage, String> {
    let mut coverage = Coverage::default();
    for (subject, level) in canonical_coverage(document, lossy_wide_integers)? {
        let subjects = match level {
            CoverageLevel::Exact => &mut coverage.exact,
            CoverageLevel::Opaque => &mut coverage.opaque,
            CoverageLevel::Lossy => &mut coverage.lossy,
            CoverageLevel::Unsupported => &mut coverage.unsupported,
        };
        subjects.insert(subject);
    }
    Ok(coverage)
}

pub fn operation_subject(operation: &Operation) -> String {
    operation_path(&operation.id)
}

pub fn type_subject(id: &str) -> String {
    format!("types.{id}")
}
