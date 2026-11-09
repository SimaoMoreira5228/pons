use serde_json::Value;

use crate::IR_VERSION;
use crate::document::DecodeError;

pub trait Migration {
    fn name(&self) -> &'static str;

    fn source_version(&self) -> &'static str;

    fn target_version(&self) -> &'static str;

    fn apply(&self, document: Value) -> Result<Value, DecodeError>;
}

#[derive(Default)]
pub struct MigrationRegistry {
    migrations: Vec<Box<dyn Migration>>,
}

struct ZeroTwoToZeroThree;

impl Migration for ZeroTwoToZeroThree {
    fn name(&self) -> &'static str {
        "ir-0.2-to-0.3"
    }

    fn source_version(&self) -> &'static str {
        "0.2"
    }

    fn target_version(&self) -> &'static str {
        IR_VERSION
    }

    fn apply(&self, mut document: Value) -> Result<Value, DecodeError> {
        document["ir_version"] = Value::String(IR_VERSION.to_owned());
        Ok(document)
    }
}

impl MigrationRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn standard() -> Self {
        Self::empty().with(ZeroTwoToZeroThree)
    }

    pub fn register(&mut self, migration: impl Migration + 'static) {
        self.migrations.push(Box::new(migration));
    }

    pub fn with(mut self, migration: impl Migration + 'static) -> Self {
        self.register(migration);
        self
    }

    pub fn migrate(
        &self,
        mut document: Value,
        from: &str,
        to: &str,
    ) -> Result<Option<Value>, DecodeError> {
        let mut current = from.to_owned();
        let mut applied = Vec::new();
        while current != to {
            let mut candidates = self
                .migrations
                .iter()
                .filter(|migration| migration.source_version() == current);
            let Some(migration) = candidates.next() else {
                return Ok(None);
            };
            if let Some(second) = candidates.next() {
                return Err(DecodeError::Migration {
                    name: migration.name().to_owned(),
                    message: format!(
                        "the registry has more than one migration from {current}: {} and {}",
                        migration.name(),
                        second.name()
                    ),
                });
            }
            if applied.iter().any(|name| name == migration.name()) {
                return Err(DecodeError::Migration {
                    name: migration.name().to_owned(),
                    message: format!("the migration loop returns to {current}"),
                });
            }
            applied.push(migration.name().to_owned());
            document = migration.apply(document)?;
            let reached = document
                .get("ir_version")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if reached != migration.target_version() {
                return Err(DecodeError::Migration {
                    name: migration.name().to_owned(),
                    message: format!(
                        "the migration moved the document to {reached:?}, not {:?}",
                        migration.target_version()
                    ),
                });
            }
            current = migration.target_version().to_owned();
        }
        Ok(Some(document))
    }
}
