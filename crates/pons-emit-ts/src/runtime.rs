use crate::{GeneratedFile, Runtime};

pub(super) const PACKAGE: &str = "@pons/runtime";

const MODULES: &[(&str, &str)] = &[
    ("index.ts", include_str!("../../../runtime/ts/index.ts")),
    ("wire.ts", include_str!("../../../runtime/ts/wire.ts")),
    ("codec.ts", include_str!("../../../runtime/ts/codec.ts")),
    (
        "validate.ts",
        include_str!("../../../runtime/ts/validate.ts"),
    ),
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

pub(super) fn files(runtime: Runtime) -> Vec<GeneratedFile> {
    if runtime == Runtime::Package {
        return Vec::new();
    }
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

pub(super) fn specifier(runtime: Runtime, vendored: &str) -> String {
    match runtime {
        Runtime::Vendor => vendored.to_owned(),
        Runtime::Package => format!("\"{PACKAGE}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::{MODULES, PACKAGE, files, normalize, specifier};
    use crate::Runtime;
    use serde_json::Value;
    use std::collections::BTreeSet;

    fn repository(relative: &str) -> String {
        std::fs::read_to_string(format!("{}/../../{relative}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn manifest() -> Value {
        serde_json::from_str(&repository("runtime/ts/package.json")).unwrap()
    }

    #[test]
    fn the_emitted_runtime_is_the_typescript_this_repository_holds() {
        for file in files(Runtime::Vendor) {
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

    #[test]
    fn the_runtime_package_root_is_the_directory_the_modules_import_from() {
        let manifest = manifest();
        assert_eq!(manifest["name"], PACKAGE, "{manifest}");
        assert_eq!(
            manifest["type"], "module",
            "the sources are ESM: {manifest}"
        );
        let exports = manifest["exports"].as_object().expect("exports");
        assert_eq!(
            exports.len(),
            1,
            "the generated code imports one subpath, so one is what the package offers: {manifest}"
        );
        let root = exports.get(".").and_then(Value::as_str).unwrap_or_else(|| {
            panic!("the root subpath is the one the generated code imports: {manifest}")
        });
        let module = root.strip_prefix("./").unwrap_or_else(|| {
            panic!("{root} names a module beside the manifest, not under a directory")
        });
        assert!(
            MODULES.iter().any(|(name, _)| *name == module),
            "{root} is a module this emitter vendors: {:?}",
            MODULES.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        );
        let toolchain: Value = serde_json::from_str(&repository("package.json")).unwrap();
        assert_eq!(
            manifest["peerDependencies"]["typescript"],
            format!(
                ">={}",
                toolchain["devDependencies"]["typescript"].as_str().unwrap()
            ),
            "a consumer compiles the TypeScript this repository compiles: {manifest}"
        );
        assert_eq!(
            manifest["engines"]["node"], ">=18",
            "the runtime calls fetch, Headers, Response, ReadableStream, and Blob as globals: \
             {manifest}"
        );
        assert_eq!(
            manifest["version"],
            env!("CARGO_PKG_VERSION"),
            "the runtime ships with the crate that vendors it: {manifest}"
        );
        let mut reaches = BTreeSet::new();
        for (_, source) in MODULES {
            for (at, _) in source.match_indices("\"../") {
                let rest = &source[at + 1..];
                reaches.insert(format!("\"{}\"", &rest[..rest.find('"').unwrap()]));
            }
        }
        assert_eq!(
            reaches,
            BTreeSet::from([
                "\"../client.js\"".to_owned(),
                "\"../errors.js\"".to_owned(),
                "\"../types.js\"".to_owned(),
            ]),
            "the runtime reaches the client only through these names, so the package root has to \
             be the directory they leave, one below the client that wrote them"
        );
    }

    #[test]
    fn the_package_mode_names_the_package_where_the_vendored_path_was() {
        assert_eq!(
            specifier(Runtime::Vendor, "./runtime.js"),
            "./runtime.js",
            "the vendored client imports the file beside it"
        );
        assert_eq!(
            specifier(Runtime::Vendor, "../runtime.js"),
            "../runtime.js",
            "an operation file imports it one directory up"
        );
        assert_eq!(
            specifier(Runtime::Package, "./runtime.js"),
            "\"@pons/runtime\""
        );
        assert_eq!(
            specifier(Runtime::Package, "../runtime.js"),
            "\"@pons/runtime\""
        );
        assert!(
            files(Runtime::Package).is_empty(),
            "the package supplies the modules, so the emitter writes none of them"
        );
        assert_eq!(
            files(Runtime::Vendor).len(),
            MODULES.len() + 1,
            "the entry point and every module, which is the file set the default emits"
        );
    }
}
