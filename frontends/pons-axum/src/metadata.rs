use pons_ir::{CredentialLocator, CredentialProvision, Doc, Name, Pagination};

pub struct OperationMeta {
    pub(crate) id: Vec<Name>,
    pub(crate) doc: Doc,
    pub(crate) provides: Option<CredentialProvision>,
    pub(crate) expires_at: Option<CredentialLocator>,
    pub(crate) semantics: Semantics,
}

#[derive(Default)]
pub(crate) struct Semantics {
    pub(crate) safe: Option<bool>,
    pub(crate) idempotent: Option<bool>,
    pub(crate) pagination: Option<Pagination>,
}

impl OperationMeta {
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        let id = id
            .split('.')
            .map(|segment| {
                let name = segment
                    .split('_')
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                assert!(!name.is_empty(), "operation id cannot be empty");
                name
            })
            .collect();
        Self {
            id,
            doc: Doc {
                summary: None,
                description: None,
                deprecated: None,
                tags: Vec::new(),
            },
            provides: None,
            expires_at: None,
            semantics: Semantics::default(),
        }
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.doc.summary = Some(summary.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.doc.description = Some(description.into());
        self
    }

    pub fn deprecated(mut self, deprecated: bool) -> Self {
        self.doc.deprecated = Some(deprecated);
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.doc.tags.push(tag.into());
        self
    }

    pub fn provides(mut self, scheme: impl Into<String>, locator: CredentialLocator) -> Self {
        self.provides = Some(CredentialProvision {
            scheme: scheme.into(),
            value: locator,
            expires_at: self.expires_at.take(),
            ext: Default::default(),
        });
        self
    }

    pub fn expires_at(mut self, locator: CredentialLocator) -> Self {
        if let Some(provision) = self.provides.as_mut() {
            provision.expires_at = Some(locator);
        } else {
            self.expires_at = Some(locator);
        }
        self
    }

    pub fn safe(mut self, safe: bool) -> Self {
        self.semantics.safe = Some(safe);
        self
    }

    pub fn idempotent(mut self, idempotent: bool) -> Self {
        self.semantics.idempotent = Some(idempotent);
        self
    }

    pub fn pagination(mut self, pagination: Pagination) -> Self {
        self.semantics.pagination = Some(pagination);
        self
    }
}
