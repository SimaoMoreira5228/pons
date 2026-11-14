use crate::IR_VERSION;
use crate::types::{Diagnostic, DiagnosticSeverity};

use super::{Document, ValidationError};

impl Document {
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if self.ir_version != IR_VERSION {
            errors.push(ValidationError::UnsupportedVersion {
                found: self.ir_version.clone(),
            });
        }
        self.validate_diagnostics("document", &self.diagnostics, &mut errors);
        for (id, definition) in &self.types {
            self.validate_diagnostics(&format!("type {id}"), &definition.diagnostics, &mut errors);
            self.validate_type(id, definition, &mut errors);
        }
        self.validate_auth_schemes(&mut errors);
        self.validate_common_errors(&mut errors);
        self.validate_operations(&mut errors);
        self.validate_directions(&mut errors);
        self.validate_credential_carriers(&mut errors);
        self.validate_extensions(&mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub(super) fn validate_diagnostics(
        &self,
        owner: &str,
        diagnostics: &[Diagnostic],
        errors: &mut Vec<ValidationError>,
    ) {
        for diagnostic in diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Error {
                errors.push(ValidationError::InvalidValue {
                    path: format!("{owner} diagnostics"),
                    message: format!(
                        "error diagnostic {}: {}",
                        diagnostic.code, diagnostic.message
                    ),
                });
            }
        }
    }
}
