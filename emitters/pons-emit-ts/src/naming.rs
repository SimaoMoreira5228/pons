use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Direction, Document, TypeDef};

use crate::options::{DateMode, TsOptions};

const RESERVED_NAMES: &[&str] = &[
    "Array",
    "Date",
    "Error",
    "Extract",
    "JSON",
    "Promise",
    "ReadonlyArray",
    "Record",
    "Uint8Array",
];

pub(crate) fn unique_names(bases: impl IntoIterator<Item = String>, extra: &[&str]) -> Vec<String> {
    let bases = bases.into_iter().collect::<Vec<_>>();
    let reserved = bases.iter().cloned().collect::<BTreeSet<_>>();
    let mut assigned = RESERVED_NAMES
        .iter()
        .map(|name| (*name).to_owned())
        .chain(extra.iter().map(|name| (*name).to_owned()))
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    bases
        .into_iter()
        .map(|base| {
            let first = seen.insert(base.clone());
            let mut suffix = if first { 1 } else { 2 };
            loop {
                let candidate = if suffix == 1 {
                    base.clone()
                } else {
                    format!("{base}{suffix}")
                };
                if (first || !reserved.contains(&candidate)) && assigned.insert(candidate.clone()) {
                    break candidate;
                }
                suffix += 1;
            }
        })
        .collect()
}

pub(crate) fn type_names(document: &Document, options: &TsOptions) -> BTreeMap<String, String> {
    let mut entries = document
        .types
        .iter()
        .map(|(id, definition)| {
            (
                pair_name(document, definition),
                is_split_half(definition),
                id.clone(),
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    let reserved = match options.date_mode {
        DateMode::String => &[][..],
        DateMode::Date => &["DateOnly"][..],
    };
    let names = unique_names(entries.iter().map(|(base, _, _)| base.clone()), reserved);
    entries
        .into_iter()
        .map(|(_, _, id)| id)
        .zip(names)
        .collect()
}

fn pair_name(document: &Document, definition: &TypeDef) -> String {
    let own = pascal(&definition.name);
    if !is_split_half(definition) {
        return own;
    }
    let base = definition
        .origin
        .as_deref()
        .and_then(|origin| document.types.get(origin))
        .filter(|partner| partner.direction == Direction::Output)
        .map_or(own, |partner| pascal(&partner.name));
    format!("{base}Input")
}

fn is_split_half(definition: &TypeDef) -> bool {
    definition.origin.is_some() && definition.direction == Direction::Input
}

pub(crate) fn pascal(words: &[String]) -> String {
    let value = words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<String>();
    identifier(&value, "Type")
}

pub(crate) fn camel(words: &[String]) -> String {
    let Some((first, rest)) = words.split_first() else {
        return "operation".into();
    };
    let mut result = first.to_lowercase();
    for word in rest {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.extend(first.to_uppercase());
            result.extend(chars);
        }
    }
    identifier(&result, "operation")
}

pub(crate) fn is_plain_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '$'
        })
}

pub(crate) fn identifier(value: &str, fallback: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return fallback.into();
    };
    let mut result = String::new();
    if first.is_ascii_alphabetic() || first == '_' || first == '$' {
        result.push(first);
    } else {
        result.push('_');
        result.push(first);
    }
    result.extend(chars.map(|character| {
        if character.is_ascii_alphanumeric() || character == '_' || character == '$' {
            character
        } else {
            '_'
        }
    }));
    if is_keyword(&result) {
        format!("{result}_")
    } else {
        result
    }
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "as"
            | "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
            | "any"
            | "boolean"
            | "number"
            | "string"
            | "symbol"
            | "type"
            | "from"
            | "of"
    )
}

pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("*/", "*\\/")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use pons_ir::{Document, NamedRef, StreamKind, TypeRef};
    use serde_json::{Value, json};

    #[test]
    fn allocates_unique_generated_declaration_names() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
        let template = document.types.get("LoginRequest").unwrap().clone();
        for (id, name) in [
            ("Collision0", vec!["collision"]),
            ("Collision1", vec!["collision", "2"]),
            ("Collision2", vec!["collision"]),
        ] {
            let mut definition = template.clone();
            definition.name = name.into_iter().map(str::to_owned).collect();
            document.types.insert(id.into(), definition);
        }
        let stream = document
            .operations
            .iter()
            .find(|operation| operation.stream.is_some())
            .unwrap()
            .clone();
        let mut ndjson = stream.clone();
        let item = match &ndjson.stream.as_ref().unwrap().kind {
            StreamKind::Sse { events } => events[0].data.clone(),
            _ => unreachable!(),
        };
        ndjson.stream.as_mut().unwrap().kind = StreamKind::Ndjson { item };
        document.operations = [
            (vec![vec!["a"]], stream.clone()),
            (vec![vec!["a", "stream"]], stream.clone()),
            (vec![vec!["b"]], ndjson),
            (vec![vec!["b", "stream"]], stream),
        ]
        .into_iter()
        .map(|(id, mut operation)| {
            operation.id = id
                .into_iter()
                .map(|segment| segment.into_iter().map(str::to_owned).collect())
                .collect();
            operation
        })
        .collect();

        let files = crate::emit(&document).unwrap();
        let types = &files
            .iter()
            .find(|file| file.path == "types.ts")
            .unwrap()
            .content;
        let members = files
            .iter()
            .filter(|file| file.path.starts_with("operations/"))
            .map(|file| (file.path.clone(), file.content.clone()))
            .collect::<Vec<_>>();
        assert_eq!(members.len(), 4, "one file per top-level member");
        for source in std::iter::once(types).chain(members.iter().map(|(_, content)| content)) {
            let declarations = source.matches("export type ").count();
            let names = source
                .lines()
                .filter_map(|line| line.strip_prefix("export type "))
                .map(|line| line.split_whitespace().next().unwrap())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(names.len(), declarations, "{source}");
        }
        for name in ["Collision", "Collision2", "Collision3"] {
            assert!(types.contains(&format!("export type {name} =")));
        }
        let declared = members
            .iter()
            .map(|(_, content)| content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for name in [
            "AEvent",
            "BItem",
            "AStreamError",
            "AStreamError2",
            "BStreamError",
            "BStreamError2",
        ] {
            assert!(
                declared.contains(&format!("export type {name} =")),
                "{name}"
            );
        }
    }

    fn carriers(definitions: &[(&str, Value)]) -> Document {
        let mut raw: Value =
            serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
        for (id, definition) in definitions {
            raw["types"][*id] = definition.clone();
        }
        serde_json::from_value(raw).unwrap()
    }

    fn carrier(name: &[&str], direction: &str, origin: Option<&str>, marker: &str) -> Value {
        json!({
            "kind": "struct",
            "name": name,
            "direction": direction,
            "origin": origin,
            "openness": "reject",
            "fields": [{
                "wire": marker,
                "ident": [marker],
                "ty": {"kind": "string"},
                "presence": "required"
            }]
        })
    }

    fn file<'a>(files: &'a [crate::GeneratedFile], path: &str) -> &'a str {
        &files.iter().find(|file| file.path == path).unwrap().content
    }

    fn aliases(document: &Document) -> BTreeMap<String, String> {
        let files = crate::emit(document).unwrap();
        let types = file(&files, "types.ts");
        let aliases = types
            .lines()
            .filter_map(|line| line.strip_prefix("export type "))
            .filter_map(|line| line.split_once(" = "))
            .map(|(name, body)| {
                (
                    name.to_owned(),
                    body.strip_suffix(';').unwrap_or(body).to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            aliases.len(),
            aliases
                .iter()
                .map(|(name, _)| name)
                .collect::<BTreeSet<_>>()
                .len(),
            "{aliases:?} declares one name twice"
        );
        aliases.into_iter().collect()
    }

    #[test]
    fn names_a_direction_pair_after_the_pair_and_never_after_the_id() {
        let mut document = carriers(&[
            (
                "router::Account",
                carrier(&["account"], "output", Some("router::Account"), "id"),
            ),
            (
                "storefront::AccountRequest",
                carrier(
                    &["account", "request"],
                    "input",
                    Some("router::Account"),
                    "email",
                ),
            ),
        ]);
        let login = &mut document.operations[0];
        login.body.as_mut().unwrap().ty = TypeRef::Named(NamedRef {
            id: "storefront::AccountRequest".into(),
            variant: None,
        });
        login.responses[0].body = Some(TypeRef::Named(NamedRef {
            id: "router::Account".into(),
            variant: None,
        }));

        let files = crate::emit(&document).unwrap();
        let types = file(&files, "types.ts");
        let auth = file(&files, "operations/auth.ts");
        assert!(
            types.contains("export type Account = { \"id\": string };"),
            "{types}"
        );
        assert!(
            types.contains("export type AccountInput = { \"email\": string };"),
            "{types}"
        );
        assert!(
            auth.contains("Types.AccountInput") && auth.contains("Types.Account"),
            "{auth}"
        );
        assert!(
            types.lines().all(|line| !line.contains('#')),
            "{types} leaks an id"
        );

        let lone = carriers(&[(
            "storefront::AccountRequest",
            carrier(&["account"], "input", Some("router::Account"), "email"),
        )]);
        let files = crate::emit(&lone).unwrap();
        assert!(
            file(&files, "types.ts").contains("export type AccountInput = { \"email\": string };"),
            "pruning the response half leaves the request half the pair's name"
        );
        assert!(!file(&files, "types.ts").contains("export type Account ="));
    }

    #[test]
    fn a_pair_that_declares_no_origin_keeps_its_own_display_name() {
        let names = aliases(&carriers(&[
            ("a::Account", carrier(&["account"], "output", None, "id")),
            ("b::Account", carrier(&["account"], "input", None, "email")),
        ]));
        assert_eq!(names["Account"], "{ \"id\": string }");
        assert_eq!(names["Account2"], "{ \"email\": string }");
        assert!(!names.contains_key("AccountInput"));
    }

    #[test]
    fn a_suffixed_alias_never_takes_the_alias_another_type_publishes() {
        let names = aliases(&carriers(&[
            ("a::Q", carrier(&["q"], "output", Some("a::Q"), "answer")),
            (
                "a::Q#input",
                carrier(&["q"], "input", Some("a::Q"), "first"),
            ),
            (
                "b::Q#input",
                carrier(&["q"], "input", Some("b::Q"), "second"),
            ),
            ("c::qInput", carrier(&["qInput"], "both", None, "published")),
        ]));
        assert_eq!(names["QInput"], "{ \"published\": string }");
        assert_eq!(names["QInput2"], "{ \"first\": string }");
        assert_eq!(names["QInput3"], "{ \"second\": string }");
    }
}
