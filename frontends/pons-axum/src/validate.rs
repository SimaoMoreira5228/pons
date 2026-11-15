use std::any::type_name;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use pons_core::{Schema, TypeCx};
use pons_ir::{Direction, Document, TypeRef, Violation, validate_value};
use serde_json::Value;

use crate::ViolationReport;
use crate::wire_value::object_value;

const INPUT: &str = "input";

pub(crate) fn request_contract<T: Schema>(
    extensions: &axum::http::Extensions,
    carrier: &str,
) -> Result<(Arc<Document>, TypeRef), ViolationReport> {
    let Some(document) = extensions.get::<Arc<Document>>() else {
        return Err(violated(
            "a request carrying the document Router::into_axum installs",
            "a request carrying none".to_owned(),
        ));
    };
    Ok((document.clone(), request_type::<T>(carrier)))
}

fn request_type<T: Schema>(carrier: &str) -> TypeRef {
    static REQUEST_TYPES: OnceLock<Mutex<BTreeMap<(String, String), TypeRef>>> = OnceLock::new();
    let (id, role) = (type_name::<T>(), carrier.to_owned());
    let key = (id.to_owned(), role.clone());
    let mut types = REQUEST_TYPES
        .get_or_init(Default::default)
        .lock()
        .expect("the request type registry is never poisoned");
    types
        .entry(key)
        .or_insert_with(|| {
            let mut cx = TypeCx::new();
            match role.as_str() {
                INPUT => T::schema(&mut cx, Direction::Input),
                role => {
                    cx.register_as(format!("{id}#{role}"), |cx| T::schema(cx, Direction::Input))
                }
            }
        })
        .clone()
}

pub(crate) fn check(
    document: &Document,
    ty: &TypeRef,
    value: &Value,
) -> Result<(), ViolationReport> {
    validate_value(document, ty, Direction::Input, value).map_err(ViolationReport::rejected)
}

pub(crate) fn read_pairs<T: serde::de::DeserializeOwned>(
    document: &Document,
    ty: &TypeRef,
    pairs: Vec<(String, String)>,
) -> Result<T, ViolationReport> {
    let (value, mut violations) = object_value(document, ty, pairs);
    if let Err(found) = validate_value(document, ty, Direction::Input, &value) {
        violations.extend(found);
    }
    if !violations.is_empty() {
        return Err(ViolationReport::rejected(violations));
    }
    serde_json::from_value(value)
        .map_err(|error| violated("a value the request type reads", error.to_string()))
}

pub(crate) fn violated(expected: &str, found: String) -> ViolationReport {
    ViolationReport::rejected(vec![Violation::Type {
        path: String::new(),
        expected: expected.to_owned(),
        found,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_value::token_value;
    use pons_ir::{InlineType, IntegerWire};
    use serde_json::json;

    fn contract() -> Document {
        serde_json::from_value(json!({
            "ir_version": "0.3",
            "info": {"title": "Validation", "version": "1"},
            "types": {
                "Filter#input": {
                    "name": ["filter"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "fields": [
                        {
                            "wire": "slug",
                            "ident": ["slug"],
                            "ty": {"kind": "string", "pattern": "^[a-z]+$"},
                            "presence": "required",
                        },
                        {
                            "wire": "limit",
                            "ident": ["limit"],
                            "ty": {
                                "kind": "int",
                                "bits": 16,
                                "signed": false,
                                "wire": "number",
                                "minimum": 1,
                                "maximum": 10,
                            },
                            "presence": "optional",
                        },
                        {
                            "wire": "tags",
                            "ident": ["tags"],
                            "ty": {
                                "kind": "array",
                                "item": {"kind": "string", "pattern": "^[a-z]+$"},
                            },
                            "presence": "optional",
                        },
                    ],
                },
                "Page#input": {
                    "name": ["page"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "fields": [
                        {
                            "wire": "cursor",
                            "ident": ["cursor"],
                            "ty": {
                                "kind": "int",
                                "bits": 32,
                                "signed": false,
                                "wire": "number",
                                "maximum": 100,
                            },
                            "presence": "required",
                        },
                    ],
                },
                "Wrapped#input": {
                    "name": ["wrapped"],
                    "direction": "input",
                    "kind": "newtype",
                    "inner": {"ref": "Filter#input"},
                },
                "Paged#input": {
                    "name": ["paged"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "flattened": [{"ref": "WrappedPage#input"}],
                },
                "WrappedPage#input": {
                    "name": ["wrapped", "page"],
                    "direction": "input",
                    "kind": "newtype",
                    "inner": {"ref": "Page#input"},
                },
                "Objects#input": {
                    "name": ["objects"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "fields": [
                        {
                            "wire": "payload",
                            "ident": ["payload"],
                            "ty": {"kind": "array", "item": {"kind": "any"}},
                            "presence": "required",
                        },
                    ],
                },
            },
        }))
        .unwrap()
    }

    fn filter() -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: "Filter#input".into(),
            variant: None,
        })
    }

    fn named(id: &str) -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: id.into(),
            variant: None,
        })
    }

    fn reported(
        contract: &Document,
        ty: &TypeRef,
        pairs: &[(&str, &str)],
    ) -> Result<Value, String> {
        let pairs = pairs
            .iter()
            .map(|(wire, text)| ((*wire).to_owned(), (*text).to_owned()))
            .collect();
        read_pairs::<Value>(contract, ty, pairs).map_err(|report| {
            report
                .violations
                .iter()
                .map(|violation| format!("{}: {}", violation.path, violation.reason))
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    fn found(pairs: &[(&str, &str)]) -> Vec<String> {
        let contract = contract();
        let pairs = pairs
            .iter()
            .map(|(wire, text)| ((*wire).to_owned(), (*text).to_owned()))
            .collect();
        read_pairs::<Value>(&contract, &filter(), pairs)
            .err()
            .map(|report| {
                report
                    .violations
                    .iter()
                    .map(|violation| format!("{}: {}", violation.path, violation.reason))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn text_tokens_are_read_in_the_wire_form_the_contract_publishes() {
        let contract = contract();
        let number = TypeRef::Inline(InlineType::Int {
            bits: 16,
            signed: false,
            wire: IntegerWire::Number,
            minimum: None,
            maximum: None,
            multiple_of: None,
        });
        let text = TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        });
        assert_eq!(token_value(&contract, &number, "7"), json!(7));
        assert_eq!(token_value(&contract, &number, "seven"), json!("seven"));
        assert_eq!(token_value(&contract, &text, "7"), json!("7"));
        assert_eq!(token_value(&contract, &filter(), "7"), json!("7"));
    }

    #[test]
    fn a_text_token_pair_is_checked_against_the_published_query_type() {
        assert_eq!(found(&[("slug", "ada")]), Vec::<String>::new());
        assert_eq!(
            found(&[("slug", "Ada")]),
            ["/slug: string does not match the pattern ^[a-z]+$"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("limit", "99")]),
            ["/limit: integer is above its maximum of 10"]
        );
        assert_eq!(
            found(&[("limit", "1")]),
            ["/slug: missing required field slug"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("page", "2")]),
            ["/page: unknown property page"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("limit", "1"), ("limit", "2")]),
            ["/limit: expected one value for a field that is not an array, found 2 values"]
        );
    }

    #[test]
    fn an_array_field_reads_every_value_the_request_spellings_carry() {
        assert_eq!(
            found(&[("slug", "ada"), ("tags", "[\"x\",\"y\"]")]),
            Vec::<String>::new()
        );

        assert_eq!(
            found(&[("slug", "ada"), ("tags", "x"), ("tags", "y")]),
            Vec::<String>::new()
        );
        assert_eq!(
            found(&[("slug", "ada"), ("tags", "x"), ("tags", "Y")]),
            ["/tags/1: string does not match the pattern ^[a-z]+$"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("tags", "[\"x\",\"Y\"]")]),
            ["/tags/1: string does not match the pattern ^[a-z]+$"]
        );
    }

    #[test]
    fn a_carrier_newtype_publishes_the_members_of_the_struct_it_wraps() {
        let contract = contract();
        assert_eq!(
            reported(
                &contract,
                &named("Wrapped#input"),
                &[("slug", "ada"), ("limit", "2")]
            ),
            Ok(json!({"slug": "ada", "limit": 2}))
        );
        assert_eq!(
            reported(
                &contract,
                &named("Wrapped#input"),
                &[("slug", "ada"), ("limit", "99")]
            ),
            Err("/limit: integer is above its maximum of 10".to_owned())
        );
    }

    #[test]
    fn a_newtype_wrapped_flattened_member_publishes_its_own_members() {
        let contract = contract();
        assert_eq!(
            reported(&contract, &named("Paged#input"), &[("cursor", "7")]),
            Ok(json!({"cursor": 7}))
        );
        assert_eq!(
            reported(&contract, &named("Paged#input"), &[("cursor", "900")]),
            Err("/cursor: integer is above its maximum of 100".to_owned())
        );
        assert_eq!(
            reported(&contract, &named("Paged#input"), &[("cursor", "seven")]),
            Err("/cursor: expected a number, found a string".to_owned())
        );
    }

    #[test]
    fn a_repeated_key_reads_the_json_a_value_carries() {
        let contract = contract();
        let value = json!({"payload": [{"a": 1}, {"b": 2}]});
        assert_eq!(
            reported(
                &contract,
                &named("Objects#input"),
                &[("payload", r#"[{"a":1},{"b":2}]"#)]
            ),
            Ok(value.clone()),
            "one token that is a JSON array carries the whole field"
        );
        assert_eq!(
            reported(
                &contract,
                &named("Objects#input"),
                &[("payload", r#"{"a":1}"#), ("payload", r#"{"b":2}"#)]
            ),
            Ok(value),
            "a repeated key reads the same value, not the text it was spelled with"
        );
    }
}
