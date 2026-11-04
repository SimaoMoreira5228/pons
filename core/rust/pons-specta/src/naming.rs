use pons_ir::Name;

pub(crate) fn words(source: &str) -> Name {
    let source = source.trim_start_matches("r#");
    let mut result = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = source.chars().collect();
    for (index, character) in chars.iter().copied().enumerate() {
        if !character.is_alphanumeric() {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            continue;
        }
        if character.is_uppercase() && !current.is_empty() {
            let previous = chars
                .get(index.wrapping_sub(1))
                .copied()
                .unwrap_or_default();
            let next = chars.get(index + 1).copied().unwrap_or_default();
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

pub(crate) fn rename(source: &str, rule: &str) -> String {
    let words = words(source);
    match rule {
        "lowercase" => words.concat(),
        "UPPERCASE" => words.concat().to_uppercase(),
        "PascalCase" => words
            .iter()
            .map(|word| capitalize(word))
            .collect::<String>(),
        "camelCase" => words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index == 0 {
                    word.clone()
                } else {
                    capitalize(word)
                }
            })
            .collect::<String>(),
        "snake_case" => words.join("_"),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_uppercase(),
        "kebab-case" => words.join("-"),
        "SCREAMING-KEBAB-CASE" => words.join("-").to_uppercase(),
        _ => source.to_owned(),
    }
}

fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}
