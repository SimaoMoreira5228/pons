use crate::GeneratedFile;

const MODULES: &[(&str, &str)] = &[
    ("index.ts", include_str!("../../../runtime/ts/index.ts")),
    ("wire.ts", include_str!("../../../runtime/ts/wire.ts")),
    ("codec.ts", include_str!("../../../runtime/ts/codec.ts")),
    ("stream.ts", include_str!("../../../runtime/ts/stream.ts")),
    (
        "credentials.ts",
        include_str!("../../../runtime/ts/credentials.ts"),
    ),
    (
        "transport.ts",
        include_str!("../../../runtime/ts/transport.ts"),
    ),
];

fn normalize(source: &str) -> String {
    source.replace("\r\n", "\n")
}

pub(super) fn files() -> Vec<GeneratedFile> {
    let mut files = vec![GeneratedFile {
        path: "runtime.ts".into(),
        content: "export * from \"./runtime/index.js\";\n".into(),
    }];
    files.extend(MODULES.iter().map(|(name, source)| GeneratedFile {
        path: format!("runtime/{name}"),
        content: normalize(source),
    }));
    files
}

#[cfg(test)]
mod tests {
    use super::{MODULES, files, normalize};

    #[test]
    fn the_emitted_runtime_is_the_typescript_this_repository_holds() {
        for file in files() {
            assert!(
                !file.content.contains('\r'),
                "{} carries a carriage return, so a CRLF checkout emits a different client",
                file.path
            );
            let Some(name) = file.path.strip_prefix("runtime/") else {
                assert_eq!(file.path, "runtime.ts");
                continue;
            };
            let source = MODULES
                .iter()
                .find(|(module, _)| *module == name)
                .unwrap_or_else(|| panic!("{} is not a module", file.path))
                .1;
            assert_eq!(file.content, *source, "{}", file.path);
        }
    }

    #[test]
    fn a_checkout_that_rewrites_the_newlines_emits_the_same_client() {
        for (name, source) in MODULES {
            let crlf = source.replace('\n', "\r\n");
            assert!(crlf.contains("\r\n"), "{name} has no newline to rewrite");
            assert_eq!(&normalize(&crlf), source, "{name}");
        }
    }
}
