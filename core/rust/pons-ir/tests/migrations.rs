use pons_ir::{
    DecodeError, Document, IR_VERSION, Migration, MigrationRegistry, decode, decode_with,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
}

struct RenameTitle;

impl Migration for RenameTitle {
    fn name(&self) -> &'static str {
        "rename-title"
    }

    fn source_version(&self) -> &'static str {
        "0.1"
    }

    fn target_version(&self) -> &'static str {
        IR_VERSION
    }

    fn apply(&self, mut document: Value) -> Result<Value, DecodeError> {
        document["ir_version"] = json!(IR_VERSION);
        document["info"]["title"] = json!("Migrated API");
        Ok(document)
    }
}

struct LeavesVersion;

impl Migration for LeavesVersion {
    fn name(&self) -> &'static str {
        "leaves-version"
    }

    fn source_version(&self) -> &'static str {
        "0.0"
    }

    fn target_version(&self) -> &'static str {
        "0.1"
    }

    fn apply(&self, document: Value) -> Result<Value, DecodeError> {
        Ok(document)
    }
}

#[test]
fn the_current_version_decodes_without_a_migration() {
    let document: Document =
        decode(include_bytes!("../../../../spec/fixtures/login.json")).unwrap();
    assert_eq!(document.ir_version, IR_VERSION);
}

#[test]
fn an_unknown_version_rejects_without_running_a_migration() {
    let mut raw = fixture();
    raw["ir_version"] = json!("0.4");
    raw["operations"][0]["method"] = json!("BREW");
    let error = decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported IR version 0.4, this build reads 0.3"
    );
}

#[test]
fn a_registered_migration_runs_before_the_current_shape_is_decoded() {
    let mut raw = fixture();
    raw["ir_version"] = json!("0.1");
    raw["info"]["title"] = json!("Old API");
    let registry = MigrationRegistry::empty().with(RenameTitle);

    let document: Document = decode_with(&serde_json::to_vec(&raw).unwrap(), &registry).unwrap();
    assert_eq!(document.ir_version, IR_VERSION);
    assert_eq!(document.info.title, "Migrated API");
}

#[test]
fn a_zero_two_document_migrates_to_the_current_version() {
    let mut raw = fixture();
    raw["ir_version"] = json!("0.2");
    let document: Document = decode_with(
        &serde_json::to_vec(&raw).unwrap(),
        &MigrationRegistry::standard(),
    )
    .unwrap();
    assert_eq!(document.ir_version, IR_VERSION);
    assert_eq!(document.info.title, raw["info"]["title"]);
}

#[test]
fn the_default_registry_has_no_migrations() {
    let mut raw = fixture();
    raw["ir_version"] = json!("0.1");
    let error = decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported IR version 0.1, this build reads 0.3"
    );
}

#[test]
fn a_migration_that_does_not_reach_its_declared_version_is_rejected() {
    let mut raw = fixture();
    raw["ir_version"] = json!("0.0");
    let registry = MigrationRegistry::empty().with(LeavesVersion);
    let error = decode_with(&serde_json::to_vec(&raw).unwrap(), &registry).unwrap_err();
    assert_eq!(
        error.to_string(),
        "migration leaves-version failed: the migration moved the document to \"0.0\", not \"0.1\""
    );
}

#[test]
fn migrations_chain_through_named_versions() {
    let mut raw = fixture();
    raw["ir_version"] = json!("0.0");
    raw["info"]["title"] = json!("Old API");
    let chain = Chain;
    let registry = MigrationRegistry::empty().with(chain).with(RenameTitle);
    let document: Document = decode_with(&serde_json::to_vec(&raw).unwrap(), &registry).unwrap();
    assert_eq!(document.ir_version, IR_VERSION);
    assert_eq!(document.info.title, "Migrated API");
}

struct Chain;

impl Migration for Chain {
    fn name(&self) -> &'static str {
        "zero-to-one"
    }

    fn source_version(&self) -> &'static str {
        "0.0"
    }

    fn target_version(&self) -> &'static str {
        "0.1"
    }

    fn apply(&self, mut document: Value) -> Result<Value, DecodeError> {
        document["ir_version"] = json!("0.1");
        Ok(document)
    }
}

#[test]
fn a_migrated_document_still_rejects_an_unknown_member_with_its_path() {
    let mut raw = fixture();
    raw["ir_version"] = json!("0.1");
    raw["legacy"] = json!(true);
    let registry = MigrationRegistry::empty().with(RenameTitle);
    let error = decode_with(&serde_json::to_vec(&raw).unwrap(), &registry).unwrap_err();
    assert_eq!(
        error.to_string(),
        "legacy: unknown field `legacy`, expected one of `ir_version`, `info`, `types`, `auth_schemes`, `common_errors`, `operations`, `ext`, `diagnostics`"
    );
}
