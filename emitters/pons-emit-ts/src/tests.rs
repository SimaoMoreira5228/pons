mod coverage;
mod documentation;
mod emission;
mod rejection;

use std::collections::BTreeMap;

use pons_ir::Document;

fn file<'a>(files: &'a [crate::GeneratedFile], path: &str) -> &'a str {
    &files.iter().find(|file| file.path == path).unwrap().content
}

fn fixture() -> Document {
    serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap()
}

fn emitted(document: &Document) -> (String, String, BTreeMap<String, String>) {
    let files = super::emit(document).unwrap();
    let operations = files
        .iter()
        .filter_map(|file| {
            file.path.strip_prefix("operations/").map(|member| {
                (
                    member.trim_end_matches(".ts").to_owned(),
                    file.content.clone(),
                )
            })
        })
        .collect();
    (
        file(&files, "types.ts").to_owned(),
        file(&files, "client.ts").to_owned(),
        operations,
    )
}
