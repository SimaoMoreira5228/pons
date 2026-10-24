use std::collections::BTreeSet;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::{
    CoverageLevel, Document, Operation, canonical_coverage, coverage_subjects, operation_path,
    strict_violations,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub exact: BTreeSet<String>,
    pub opaque: BTreeSet<String>,
    pub lossy: BTreeSet<String>,
    pub unsupported: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ValidationCoverage {
    pub exact: BTreeSet<String>,
    pub lossy: BTreeSet<String>,
    pub unsupported: BTreeSet<String>,
}

impl ValidationCoverage {
    pub fn levels(&self) -> impl Iterator<Item = (String, CoverageLevel)> {
        [
            (&self.exact, CoverageLevel::Exact),
            (&self.lossy, CoverageLevel::Lossy),
            (&self.unsupported, CoverageLevel::Unsupported),
        ]
        .into_iter()
        .flat_map(|(subjects, level)| subjects.iter().map(move |subject| (subject.clone(), level)))
    }

    pub fn strict_violations(&self) -> Vec<(&'static str, Vec<String>)> {
        strict_violations(self.levels(), false)
    }

    pub fn strict_violation_message(&self) -> Option<String> {
        let violations: Vec<String> = self
            .strict_violations()
            .into_iter()
            .map(|(level, subjects)| {
                format!(
                    "{level} validation coverage in strict mode: {}",
                    subjects.join(", ")
                )
            })
            .collect();
        (!violations.is_empty()).then(|| violations.join("; "))
    }

    pub fn exact_for_document(document: &Document) -> Self {
        ValidationCoverage {
            exact: coverage_subjects(document),
            ..ValidationCoverage::default()
        }
    }
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

impl<'de> Deserialize<'de> for ValidationCoverage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(default)]
            exact: Vec<String>,
            #[serde(default)]
            lossy: Vec<String>,
            #[serde(default)]
            unsupported: Vec<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(ValidationCoverage {
            exact: coverage_set("exact", raw.exact)?,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    #[test]
    fn validation_coverage_rejects_duplicate_and_unknown_subjects() {
        for level in ["exact", "lossy", "unsupported"] {
            let mut raw = serde_json::Map::new();
            raw.insert(level.into(), json!(["types.User", "types.User"]));
            let error =
                serde_json::from_value::<ValidationCoverage>(serde_json::Value::Object(raw))
                    .unwrap_err();
            assert!(
                error.to_string().contains("more than once"),
                "{level}: {error}"
            );
        }
        let error =
            serde_json::from_value::<ValidationCoverage>(json!({"exact": [], "opaque": []}))
                .unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn validation_coverage_walks_its_three_levels() {
        let coverage = ValidationCoverage {
            exact: ["types.Exact".into()].into_iter().collect(),
            lossy: ["types.Lossy".into()].into_iter().collect(),
            unsupported: ["types.Unsupported".into()].into_iter().collect(),
        };
        assert_eq!(
            coverage.levels().collect::<BTreeMap<_, _>>(),
            BTreeMap::from([
                ("types.Exact".to_owned(), CoverageLevel::Exact),
                ("types.Lossy".to_owned(), CoverageLevel::Lossy),
                ("types.Unsupported".to_owned(), CoverageLevel::Unsupported),
            ])
        );
    }

    #[test]
    fn exact_validation_coverage_lists_every_document_subject() {
        let document: Document =
            serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
        let coverage = ValidationCoverage::exact_for_document(&document);
        assert_eq!(coverage.exact, coverage_subjects(&document));
        assert!(coverage.lossy.is_empty());
        assert!(coverage.unsupported.is_empty());
        assert_eq!(
            serde_json::to_value(&coverage).unwrap(),
            json!({
                "exact": [
                    "operations.auth.login",
                    "operations.events.watch",
                    "types.EventQuery",
                    "types.LoginError",
                    "types.LoginRequest",
                    "types.User"
                ],
                "lossy": [],
                "unsupported": []
            })
        );
    }

    #[test]
    fn validation_strict_gating_names_lossy_and_unsupported_subjects() {
        assert!(ValidationCoverage::default().strict_violations().is_empty());
        assert_eq!(
            ValidationCoverage::default().strict_violation_message(),
            None
        );

        let coverage = ValidationCoverage {
            lossy: ["types.LoginError".into()].into_iter().collect(),
            unsupported: ["operations.auth.login".into()].into_iter().collect(),
            ..ValidationCoverage::default()
        };
        assert_eq!(
            coverage.strict_violations(),
            vec![
                ("unsupported", vec!["operations.auth.login".to_owned()]),
                ("lossy", vec!["types.LoginError".to_owned()]),
            ]
        );
        let message = coverage.strict_violation_message().unwrap();
        assert!(
            message
                .contains("unsupported validation coverage in strict mode: operations.auth.login"),
            "{message}"
        );
        assert!(
            message.contains("lossy validation coverage in strict mode: types.LoginError"),
            "{message}"
        );
    }
}
