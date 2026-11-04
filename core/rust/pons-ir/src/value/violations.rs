use serde_json::Value;

use super::Validator;
use crate::violation::{Violation, describe};

impl<'a> Validator<'a> {
    pub(super) fn reject(&mut self, violation: Violation) {
        self.violations.push(violation);
    }

    pub(super) fn type_violation(&mut self, path: &str, expected: &str, found: &Value) {
        self.reject(Violation::Type {
            path: path.to_owned(),
            expected: expected.to_owned(),
            found: describe(found),
        });
    }

    pub(super) fn content_violation(&mut self, path: &str, expected: &str, found: &Value) {
        self.reject(Violation::Type {
            path: path.to_owned(),
            expected: expected.to_owned(),
            found: found.to_string(),
        });
    }

    pub(super) fn expect(
        &mut self,
        path: &str,
        expected: &str,
        value: &Value,
        shape: fn(&Value) -> bool,
    ) -> bool {
        if shape(value) {
            true
        } else {
            self.type_violation(path, expected, value);
            false
        }
    }
}

pub(super) fn child(path: &str, token: &str) -> String {
    format!("{path}/{}", token.replace('~', "~0").replace('/', "~1"))
}
