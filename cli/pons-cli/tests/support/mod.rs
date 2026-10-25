#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

pub(crate) const FIXTURE: &str = include_str!("../../../../spec/fixtures/login.json");
pub(crate) const FIXTURE_DIRECTORY: &str = "../../spec/fixtures";

pub(crate) fn pons() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pons"))
}

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_DIRECTORY)
        .join(name)
}

pub(crate) fn fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).unwrap()
}

pub(crate) fn wide_number_input(directory: &Path) -> PathBuf {
    let path = directory.join("api.json");
    fs::write(&path, fixture("lossy.json")).unwrap();
    path
}

pub(crate) fn lossy_openapi_input(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("api.json");
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["wire"] =
        serde_json::json!("number");
    contract["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["bits"] =
        serde_json::json!(64);
    fs::write(&path, serde_json::to_vec(&contract).unwrap()).unwrap();
    path
}

pub(crate) fn custom_auth_input(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("custom-api.json");
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["auth_schemes"]["tenant"] =
        serde_json::json!({ "kind": "custom", "description": "tenant handshake" });
    fs::write(&path, serde_json::to_vec(&contract).unwrap()).unwrap();
    path
}

pub(crate) fn rejecting_openness_input() -> String {
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    for id in ["User", "LoginRequest"] {
        contract["types"][id]["openness"] = serde_json::json!("reject");
    }
    serde_json::to_string(&contract).unwrap()
}

pub(crate) const PRESERVED_SUBJECTS: [&str; 4] = [
    "operations.auth.login",
    "operations.events.watch",
    "types.LoginRequest",
    "types.User",
];

pub(crate) fn coverage_for(target: &str, input: &str) -> Value {
    coverage_in(target, input, false)
        .unwrap_or_else(|error| panic!("{target} refused the document: {error}"))
}

pub(crate) fn coverage_in(target: &str, input: &str, lossy: bool) -> Result<Value, String> {
    let directory = tempfile::tempdir().unwrap();
    let output_dir = directory.path().join("generated");
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, input).unwrap();
    let mut command = pons();
    command
        .args(["emit", "--target", target])
        .arg(&input_path)
        .arg(&output_dir)
        .args(["--coverage", "coverage.json"]);
    if lossy {
        command.arg("--lossy");
    }
    let output = command.output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(stderr
            .trim()
            .strip_prefix("pons: ")
            .unwrap_or(stderr.trim())
            .to_owned());
    }
    let reported = fs::read_to_string(output_dir.join("coverage.json")).unwrap();
    Ok(serde_json::from_str(&reported).unwrap())
}

pub(crate) fn coverage_levels(report: &Value) -> BTreeMap<String, pons_ir::CoverageLevel> {
    let coverage = &report["coverage"];
    let mut levels = BTreeMap::new();
    for (level, subjects) in [
        (pons_ir::CoverageLevel::Exact, &coverage["exact"]),
        (pons_ir::CoverageLevel::Opaque, &coverage["opaque"]),
        (pons_ir::CoverageLevel::Lossy, &coverage["lossy"]),
        (
            pons_ir::CoverageLevel::Unsupported,
            &coverage["unsupported"],
        ),
    ] {
        for subject in subjects.as_array().unwrap() {
            let subject = subject.as_str().unwrap();
            assert!(
                levels.insert(subject.to_owned(), level).is_none(),
                "{subject}"
            );
        }
    }
    levels
}

pub(crate) const BUILT_INS: [&str; 2] = ["ts", "openapi"];

#[derive(Clone, Copy)]
pub(crate) struct Levels {
    exact: &'static [&'static str],
    opaque: &'static [&'static str],
    lossy: &'static [&'static str],
    unsupported: &'static [&'static str],
}

impl Levels {
    pub(crate) fn levels(&self) -> BTreeMap<String, pons_ir::CoverageLevel> {
        [
            (pons_ir::CoverageLevel::Exact, self.exact),
            (pons_ir::CoverageLevel::Opaque, self.opaque),
            (pons_ir::CoverageLevel::Lossy, self.lossy),
            (pons_ir::CoverageLevel::Unsupported, self.unsupported),
        ]
        .into_iter()
        .flat_map(|(level, subjects)| {
            subjects
                .iter()
                .map(move |subject| ((*subject).to_owned(), level))
        })
        .collect()
    }
}

pub(crate) const PRESERVED_LEVELS: Levels = Levels {
    exact: &["types.EventQuery", "types.LoginError"],
    opaque: &PRESERVED_SUBJECTS,
    lossy: &[],
    unsupported: &[],
};

pub(crate) const EVERY_SUBJECT_EXACT: Levels = Levels {
    exact: &[
        "operations.auth.login",
        "operations.events.watch",
        "types.EventQuery",
        "types.LoginError",
        "types.LoginRequest",
        "types.User",
    ],
    opaque: &[],
    lossy: &[],
    unsupported: &[],
};

pub(crate) const WIDE_INTEGER_LOSSY: Levels = Levels {
    exact: &["types.EventQuery", "types.LoginError"],
    opaque: &["types.LoginRequest"],
    lossy: &[
        "operations.auth.login",
        "operations.events.watch",
        "types.User",
    ],
    unsupported: &[],
};

pub(crate) struct Row {
    pub(crate) file: &'static str,
    pub(crate) carries: &'static str,
    pub(crate) strict: [Levels; 2],
    pub(crate) lossy: [Levels; 2],
}

pub(crate) const FIXTURES: [Row; 4] = [
    Row {
        file: "login.json",
        carries: "opaque",
        strict: [PRESERVED_LEVELS; 2],
        lossy: [PRESERVED_LEVELS; 2],
    },
    Row {
        file: "exact.json",
        carries: "exact",
        strict: [EVERY_SUBJECT_EXACT; 2],
        lossy: [EVERY_SUBJECT_EXACT; 2],
    },
    Row {
        file: "lossy.json",
        carries: "lossy",
        strict: [PRESERVED_LEVELS; 2],
        lossy: [WIDE_INTEGER_LOSSY; 2],
    },
    Row {
        file: "unsupported.json",
        carries: "unsupported",
        strict: [PRESERVED_LEVELS; 2],
        lossy: [PRESERVED_LEVELS; 2],
    },
];

pub(crate) const STUB_RESPONSE: &str = r#"{"protocol":3,"files":[{"path":"acme/__init__.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}"#;

#[cfg(target_os = "linux")]
pub(crate) fn stub_manifest(option_schema: &str) -> String {
    let mut manifest = String::from(
        r#"{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"custom_auth":true,"flattened":true,"wide_integers":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}"#,
    );
    if !option_schema.is_empty() {
        manifest.push_str(",\"option_schema\":");
        manifest.push_str(option_schema);
    }
    manifest.push('}');
    manifest
}

#[cfg(target_os = "linux")]
pub(crate) fn stub(bin: &Path, target: &str, manifest: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(bin).unwrap();
    let path = bin.join(format!("pons-emit-{target}"));
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nif [ \"$1\" = describe ]; then\n  printf '%s' '{manifest}'\nelif [ \"$1\" = emit ]; then\n  cat > \"$PONS_REQUEST\"\n  printf '%s' '{STUB_RESPONSE}'\nfi\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(target_os = "linux")]
pub(crate) fn on_path(bin: &Path) -> std::ffi::OsString {
    std::env::join_paths(
        std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .unwrap()
}
