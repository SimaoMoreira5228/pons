use std::fs;
use std::process::ExitCode;

use pons_ir::{Document, decode, equivalent, equivalent_ignoring_request_rejections};

const IGNORE_REJECTIONS: &str = "--ignore-request-rejections";

fn main() -> ExitCode {
    let mut ignore_request_rejections = false;
    let mut paths = Vec::new();
    for argument in std::env::args_os().skip(1) {
        if argument == IGNORE_REJECTIONS {
            ignore_request_rejections = true;
        } else {
            paths.push(argument);
        }
    }
    let [left, right] = paths.as_slice() else {
        eprintln!("usage: pons-conformance [{IGNORE_REJECTIONS}] <left.json> <right.json>");
        return ExitCode::from(2);
    };
    let left = match fs::read(left) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("cannot read {}: {error}", left.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };
    let right = match fs::read(right) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("cannot read {}: {error}", right.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };
    match compare(&left, &right, ignore_request_rejections) {
        Ok(()) => {
            println!("the documents are equivalent");
            ExitCode::SUCCESS
        }
        Err(report) => {
            print!("{report}");
            ExitCode::FAILURE
        }
    }
}

fn compare(left: &[u8], right: &[u8], ignore_request_rejections: bool) -> Result<(), String> {
    let left =
        decode(left).map_err(|error| format!("the left document does not decode: {error}\n"))?;
    let right =
        decode(right).map_err(|error| format!("the right document does not decode: {error}\n"))?;
    report(&left, &right, ignore_request_rejections)
}

fn report(
    left: &Document,
    right: &Document,
    ignore_request_rejections: bool,
) -> Result<(), String> {
    let compared = if ignore_request_rejections {
        equivalent_ignoring_request_rejections(left, right)
    } else {
        equivalent(left, right)
    };
    let differences = match compared {
        Ok(()) => return Ok(()),
        Err(differences) => differences,
    };
    let mut text = format!("{} mismatch(es):\n", differences.len());
    for difference in differences {
        text.push_str(&difference.to_string());
        text.push('\n');
    }
    Err(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../spec/fixtures/exact.json").to_vec()
    }

    fn with_request_rejection() -> Vec<u8> {
        let mut value: Value = serde_json::from_slice(&fixture()).expect("the fixture is JSON");
        let errors = value["operations"][0]["errors"]
            .as_array_mut()
            .expect("the operation has errors");
        errors.push(serde_json::json!({
            "status": 422,
            "name": ["request", "rejected"],
            "content": "json",
            "body": {"ref": "User"},
            "retryable": false
        }));
        serde_json::to_vec(&value).expect("a JSON value serializes")
    }

    #[test]
    fn equal_documents_are_equivalent() {
        let document = fixture();
        assert_eq!(compare(&document, &document, false), Ok(()));
        assert_eq!(compare(&document, &document, true), Ok(()));
    }

    #[test]
    fn injected_request_rejections_need_the_explicit_flag() {
        let left = fixture();
        let right = with_request_rejection();
        assert!(compare(&left, &right, false).is_err());
        assert_eq!(compare(&left, &right, true), Ok(()));
    }

    #[test]
    fn a_changed_field_names_the_path_and_both_values() {
        let left = fixture();
        let mut value: Value = serde_json::from_slice(&left).expect("the fixture is JSON");
        value["types"]["User"]["fields"][0]["wire"] = Value::String("identifier".into());
        let right = serde_json::to_vec(&value).expect("a JSON value serializes");

        let report = compare(&left, &right, false).expect_err("the wire names differ");
        assert!(report.contains("/types/user (Output)/fields"), "{report}");
        assert!(report.contains("\"id\""), "{report}");
        assert!(report.contains("\"identifier\""), "{report}");
    }
}
