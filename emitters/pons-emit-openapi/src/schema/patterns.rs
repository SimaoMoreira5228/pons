use serde_json::{Value, json};

use crate::EmitError;

pub(super) fn stated_pattern(pattern: &str) -> Result<Value, EmitError> {
    if compiles(pattern) {
        Ok(json!(pattern))
    } else {
        Err(EmitError(format!(
            "the pattern {pattern:?} cannot be stated because it is not a regular expression the document and the TypeScript client can both compile"
        )))
    }
}

fn compiles(pattern: &str) -> bool {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut groups: Vec<char> = Vec::new();
    let mut at = 0;

    let mut repeatable = false;
    while at < chars.len() {
        match chars[at] {
            '\\' => {
                at += 1;
                if at == chars.len() {
                    return false;
                }
                at += 1;
                repeatable = true;
            }
            '[' => {
                at += 1;
                if chars.get(at) == Some(&'^') {
                    at += 1;
                }
                loop {
                    match chars.get(at) {
                        None => return false,
                        Some(']') => {
                            at += 1;
                            break;
                        }
                        Some('\\') => {
                            at += 1;
                            if at == chars.len() {
                                return false;
                            }
                            at += 1;
                        }
                        Some(_) => at += 1,
                    }
                }
                repeatable = true;
            }
            '(' => {
                at += 1;
                if chars.get(at) == Some(&'?')
                    && matches!(chars.get(at + 1), Some(':' | '=' | '!' | '<' | 'P'))
                {
                    at += 2;
                }
                groups.push('(');
                repeatable = false;
            }
            ')' => {
                if groups.pop() != Some('(') {
                    return false;
                }
                at += 1;
                repeatable = true;
            }
            '{' => {
                if !repeatable {
                    return false;
                }
                at += 1;
                let digits = at;
                while chars.get(at).is_some_and(char::is_ascii_digit) {
                    at += 1;
                }
                if at == digits {
                    return false;
                }
                if chars.get(at) == Some(&',') {
                    at += 1;
                    while chars.get(at).is_some_and(char::is_ascii_digit) {
                        at += 1;
                    }
                }
                if chars.get(at) != Some(&'}') {
                    return false;
                }
                at += 1;
                if chars.get(at) == Some(&'?') {
                    at += 1;
                }
                repeatable = false;
            }
            '}' => return false,
            '*' | '+' | '?' => {
                if !repeatable {
                    return false;
                }
                at += 1;
                if chars.get(at) == Some(&'?') {
                    at += 1;
                }
                repeatable = false;
            }
            '|' => {
                at += 1;
                repeatable = false;
            }
            _ => {
                at += 1;
                repeatable = true;
            }
        }
    }
    groups.is_empty()
}

#[cfg(test)]
mod tests {
    use pons_ir::{InlineType, TypeKind, TypeRef};
    use serde_json::{Value, json};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
    }

    fn patterned(pattern: &str) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: Some(pattern.into()),
        })
    }

    #[test]
    fn refuses_a_pattern_the_document_cannot_compile() {
        fn id_pattern(document: &mut pons_ir::Document, pattern: &str) {
            let TypeKind::Struct { fields, .. } =
                &mut document.types.get_mut("User").unwrap().definition
            else {
                unreachable!()
            };
            fields[0].ty = patterned(pattern);
        }
        let mut document = fixture();

        id_pattern(&mut document, "[a-");
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("[a-"), "{error}");

        id_pattern(&mut document, "a{");
        assert!(document.validate().is_ok(), "the document itself is legal");
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("a{"), "{error}");

        id_pattern(&mut document, "^[a-z0-9_-]{2,}$");
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["components"]["schemas"]["User"]["properties"]["id"]["pattern"],
            json!("^[a-z0-9_-]{2,}$")
        );
    }
}
