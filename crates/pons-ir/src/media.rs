use std::collections::BTreeMap;

pub fn media_type_matches(actual: &str, expected: &str) -> bool {
    if media_type_base(actual) != media_type_base(expected) {
        return false;
    }
    let actual_parameters = actual
        .split(';')
        .skip(1)
        .map(normalize_parameter)
        .collect::<BTreeMap<_, _>>();
    expected
        .split(';')
        .skip(1)
        .map(normalize_parameter)
        .all(|parameter| actual_parameters.get(&parameter.0) == Some(&parameter.1))
}

pub fn is_valid_media_type(value: &str) -> bool {
    let (base, has_parameters) = value
        .split_once(';')
        .map_or((value, false), |(base, _)| (base, true));
    let base = if has_parameters {
        base.trim_end_matches(is_ows)
    } else {
        base
    };
    let valid_base = base
        .split_once('/')
        .is_some_and(|(kind, subtype)| token(kind) && token(subtype));
    let valid_parameters = value.split(';').skip(1).all(|parameter| {
        let parameter = parameter.trim_matches(is_ows);
        parameter.split_once('=').is_some_and(|(name, value)| {
            let name = name.trim_matches(is_ows);
            let value = value.trim_matches(is_ows);
            token(name) && !value.is_empty() && !value.chars().any(char::is_control)
        })
    });
    valid_base
        && valid_parameters
        && !value
            .chars()
            .any(|character| character.is_control() && !is_ows(character))
}

fn media_type_base(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase()
}

fn normalize_parameter(value: &str) -> (String, String) {
    let normalized = value.trim().to_lowercase();
    let Some((name, value)) = normalized.split_once('=') else {
        return (normalized, String::new());
    };
    (
        name.trim_matches(is_ows).to_owned(),
        value.trim_matches(is_ows).to_owned(),
    )
}

fn is_ows(character: char) -> bool {
    matches!(character, ' ' | '\t')
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '!' | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '.'
                        | '^'
                        | '_'
                        | '`'
                        | '|'
                        | '~'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::{is_valid_media_type, media_type_matches};

    #[test]
    fn media_type_validation_allows_ows_before_parameters() {
        assert!(is_valid_media_type("application/json ; charset=utf-8"));
        assert!(is_valid_media_type(
            "application/json\t;\tcharset\t=\tutf-8"
        ));
        assert!(!is_valid_media_type("application /json; charset=utf-8"));
        assert!(!is_valid_media_type("application/json;"));
        assert!(!is_valid_media_type("application/json; charset=\n"));
    }

    #[test]
    fn media_type_matching_uses_runtime_normalization() {
        assert!(media_type_matches(
            "Application/PDF ; Version=ONE; Extra=Value",
            "application/pdf; version=one"
        ));
        assert!(media_type_matches(
            "application/pdf; version=one",
            "APPLICATION/PDF; VERSION=ONE"
        ));
        assert!(!media_type_matches(
            "application/pdf; version=two",
            "application/pdf; version=one"
        ));
        assert!(!media_type_matches(
            "application/pdf",
            "application/pdf; version=one"
        ));
        assert!(media_type_matches(
            "application/pdf ; version = 1",
            "application/pdf;version=1"
        ));
    }
}
