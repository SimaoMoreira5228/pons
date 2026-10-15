use std::ffi::OsString;
use std::fs;
use std::process::ExitCode;

use pons_ir::{Document, decode, equivalent};

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let [left, right] = arguments.as_slice() else {
        eprintln!("usage: pons-conformance <left.json> <right.json>");
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
    match compare(&left, &right) {
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

fn compare(left: &[u8], right: &[u8]) -> Result<(), String> {
    let left =
        decode(left).map_err(|error| format!("the left document does not decode: {error}\n"))?;
    let right =
        decode(right).map_err(|error| format!("the right document does not decode: {error}\n"))?;
    report(&left, &right)
}

fn report(left: &Document, right: &Document) -> Result<(), String> {
    let differences = match equivalent(left, right) {
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
        include_bytes!("../../../fixtures/exact.json").to_vec()
    }

    #[test]
    fn equal_documents_are_equivalent() {
        let document = fixture();
        assert_eq!(compare(&document, &document), Ok(()));
    }

    #[test]
    fn a_changed_field_names_the_path_and_both_values() {
        let left = fixture();
        let mut value: Value = serde_json::from_slice(&left).expect("the fixture is JSON");
        value["types"]["User"]["fields"][0]["wire"] = Value::String("identifier".into());
        let right = serde_json::to_vec(&value).expect("a JSON value serializes");

        let report = compare(&left, &right).expect_err("the wire names differ");
        assert!(report.contains("/types/user (Output)/fields"), "{report}");
        assert!(report.contains("\"id\""), "{report}");
        assert!(report.contains("\"identifier\""), "{report}");
    }
}
