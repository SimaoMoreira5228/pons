use pons_ir::Name;

pub(crate) fn words(source: &str) -> Name {
    let mut result = Vec::new();
    let mut current = String::new();
    let characters: Vec<char> = source.chars().collect();
    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_alphanumeric() {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            continue;
        }
        if character.is_uppercase() && !current.is_empty() {
            let previous = characters
                .get(index.wrapping_sub(1))
                .copied()
                .unwrap_or_default();
            let next = characters.get(index + 1).copied().unwrap_or_default();
            if previous.is_lowercase() || next.is_lowercase() && previous.is_uppercase() {
                result.push(std::mem::take(&mut current));
            }
        }
        current.extend(character.to_lowercase());
    }
    if !current.is_empty() {
        result.push(current);
    }
    if result.is_empty() {
        result.push("value".to_owned());
    }
    result
}

pub(crate) fn slug(source: &str) -> String {
    let mut result = String::new();
    for character in source.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else if matches!(character, '-' | '_') {
            result.push(character);
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let trimmed = result.trim_matches('-');
    if trimmed.is_empty() {
        "value".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub(crate) fn label(id: &[Name]) -> String {
    id.iter()
        .map(|part| part.join("_"))
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn extensions(
    object: &serde_json::Map<String, serde_json::Value>,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    object
        .iter()
        .filter(|(key, _)| key.starts_with("x-"))
        .map(|(key, value)| {
            (
                format!("openapi.x.{}/v1", slug(&key["x-".len()..])),
                value.clone(),
            )
        })
        .collect()
}
