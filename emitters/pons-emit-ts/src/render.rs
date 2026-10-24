use pons_ir::Doc;

use crate::naming::one_line;

pub(crate) fn string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

pub(crate) fn jsdoc(doc: Option<&Doc>) -> String {
    let Some(doc) = doc else {
        return String::new();
    };
    let mut parts = [doc.summary.as_deref(), doc.description.as_deref()]
        .into_iter()
        .flatten()
        .map(one_line)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if doc.deprecated == Some(true) {
        parts.push(String::from("@deprecated"));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("/** {} */", parts.join(" "))
}

pub(crate) fn object_type(members: &[(Option<&Doc>, String)]) -> String {
    if !members.iter().any(|(doc, _)| doc.is_some()) {
        return format!(
            "{{ {} }}",
            members
                .iter()
                .map(|(_, member)| member.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let members = members
        .iter()
        .map(|(doc, member)| match jsdoc(*doc) {
            comment if comment.is_empty() => format!("  {member};"),
            comment => format!("  {comment}\n  {member};"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{{\n{members}\n}}")
}

pub(crate) fn union_type(members: &[(Option<&Doc>, String)]) -> String {
    if !members.iter().any(|(doc, _)| doc.is_some()) {
        return members
            .iter()
            .map(|(_, member)| member.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
    }
    members
        .iter()
        .map(|(doc, member)| match jsdoc(*doc) {
            comment if comment.is_empty() => member.clone(),
            comment => format!("{comment}\n  {member}"),
        })
        .collect::<Vec<_>>()
        .join("\n  | ")
}
