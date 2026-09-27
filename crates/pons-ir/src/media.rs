pub fn is_valid_media_type(value: &str) -> bool {
    let base = value.split(';').next().unwrap_or_default();
    let valid_base = base
        .split_once('/')
        .is_some_and(|(kind, subtype)| token(kind) && token(subtype));
    let valid_parameters = value.split(';').skip(1).all(|parameter| {
        let parameter = parameter.trim();
        parameter.split_once('=').is_some_and(|(name, value)| {
            token(name.trim()) && !value.trim().is_empty() && !value.chars().any(char::is_control)
        })
    });
    valid_base && valid_parameters && !value.chars().any(char::is_control)
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
