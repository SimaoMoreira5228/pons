use std::collections::BTreeMap;

use pons_ir::{
    AuthRequirement, CredentialProvision, Diagnostic, Name, OperationSemantics,
    validate_extension_key,
};
use serde_json::{Map, Value};

use crate::document::push_dropped;
use crate::security::Schemes;

#[derive(Default)]
pub(crate) struct OperationMetadata {
    pub(crate) id: Option<Vec<Name>>,
    pub(crate) auth: Option<AuthRequirement>,
    pub(crate) semantics: Option<OperationSemantics>,
    pub(crate) provides: Option<CredentialProvision>,
    pub(crate) errors_inherit: Option<bool>,
    pub(crate) ext: Option<BTreeMap<String, Value>>,
    findings: Vec<String>,
}

impl OperationMetadata {
    pub(crate) fn read(object: &Map<String, Value>, schemes: &Schemes) -> Self {
        let mut metadata = Self::default();
        let Some(value) = object.get("x-pons") else {
            return metadata;
        };
        let Some(stored) = value.as_object() else {
            metadata
                .findings
                .push("the x-pons member is not an object, so it is ignored".into());
            return metadata;
        };
        metadata.read_id(stored);
        metadata.read_auth(stored, schemes);
        metadata.read_semantics(stored);
        metadata.read_provides(stored, schemes);
        metadata.read_errors_inherit(stored);
        metadata.read_ext(stored);
        metadata.read_stream(stored);
        metadata
    }

    pub(crate) fn report(&self, operation: &str, diagnostics: &mut Vec<Diagnostic>) {
        for finding in &self.findings {
            push_dropped(diagnostics, Some(operation), finding.clone());
        }
    }

    fn read_id(&mut self, stored: &Map<String, Value>) {
        let Some(value) = stored.get("id") else {
            return;
        };
        match serde_json::from_value::<Vec<Name>>(value.clone()) {
            Ok(id) if valid_id(&id) => self.id = Some(id),
            Ok(_) => self.findings.push(
                "the x-pons id member has an empty name segment, so the operation id comes from operationId or the path"
                    .into(),
            ),
            Err(error) => self.findings.push(format!(
                "the x-pons id member is malformed ({error}), so the operation id comes from operationId or the path"
            )),
        }
    }

    fn read_auth(&mut self, stored: &Map<String, Value>, schemes: &Schemes) {
        let Some(value) = stored.get("auth") else {
            return;
        };
        match serde_json::from_value::<AuthRequirement>(value.clone()) {
            Ok(mut auth) => match resolve_requirement(&mut auth, schemes) {
                Ok(()) => self.auth = Some(auth),
                Err(scheme) => self.findings.push(format!(
                    "the x-pons auth member names the security scheme {scheme:?}, which the document does not declare, so the requirement comes from the security field"
                )),
            },
            Err(error) => self.findings.push(format!(
                "the x-pons auth member is malformed ({error}), so the requirement comes from the security field"
            )),
        }
    }

    fn read_semantics(&mut self, stored: &Map<String, Value>) {
        let Some(value) = stored.get("semantics") else {
            return;
        };
        match serde_json::from_value::<OperationSemantics>(value.clone()) {
            Ok(semantics) => self.semantics = Some(semantics),
            Err(error) => self.findings.push(format!(
                "the x-pons semantics member is malformed ({error}), so the semantics come from the HTTP method"
            )),
        }
    }

    fn read_provides(&mut self, stored: &Map<String, Value>, schemes: &Schemes) {
        let Some(value) = stored.get("provides") else {
            return;
        };
        if value.is_null() {
            return;
        }
        match serde_json::from_value::<CredentialProvision>(value.clone()) {
            Ok(mut provides) => match schemes.resolve(&provides.scheme) {
                Some(scheme) => {
                    provides.scheme = scheme;
                    self.provides = Some(provides);
                }
                None => self.findings.push(format!(
                    "the x-pons provides member names the security scheme {:?}, which the document does not declare, so the credential provision is dropped",
                    provides.scheme
                )),
            },
            Err(error) => self.findings.push(format!(
                "the x-pons provides member is malformed ({error}), so the credential provision is dropped"
            )),
        }
    }

    fn read_errors_inherit(&mut self, stored: &Map<String, Value>) {
        let Some(value) = stored.get("errors_inherit") else {
            return;
        };
        match value.as_bool() {
            Some(value) => self.errors_inherit = Some(value),
            None => self.findings.push(
                "the x-pons errors_inherit member is not a boolean, so the operation does not inherit common errors"
                    .into(),
            ),
        }
    }

    fn read_ext(&mut self, stored: &Map<String, Value>) {
        let Some(value) = stored.get("ext") else {
            return;
        };
        let Some(ext) = value.as_object() else {
            self.findings.push(
                "the x-pons ext member is not an object, so the operation extensions come from its x-* members"
                    .into(),
            );
            return;
        };
        let mut kept = BTreeMap::new();
        for (key, value) in ext {
            match validate_extension_key(key) {
                Ok(()) => {
                    kept.insert(key.clone(), value.clone());
                }
                Err(reason) => self.findings.push(format!(
                    "the x-pons ext key {key:?} is dropped because {reason}"
                )),
            }
        }
        self.ext = Some(kept);
    }

    fn read_stream(&mut self, stored: &Map<String, Value>) {
        if stored.get("stream").is_some_and(|stream| !stream.is_null()) {
            self.findings.push(
                "the x-pons stream metadata was not restored, because an OpenAPI response cannot describe the stream"
                    .into(),
            );
        }
    }
}

fn valid_id(id: &[Name]) -> bool {
    !id.is_empty()
        && id
            .iter()
            .all(|part| !part.is_empty() && part.iter().all(|word| !word.is_empty()))
}

fn resolve_requirement(auth: &mut AuthRequirement, schemes: &Schemes) -> Result<(), String> {
    for alternative in &mut auth.alternatives {
        for scheme in alternative.iter_mut() {
            *scheme = schemes.resolve(scheme).ok_or_else(|| scheme.clone())?;
        }
    }
    let mut scopes = BTreeMap::new();
    for (scheme, declared) in std::mem::take(&mut auth.scopes) {
        let resolved = schemes.resolve(&scheme).ok_or(scheme)?;
        scopes.insert(resolved, declared);
    }
    auth.scopes = scopes;
    Ok(())
}
