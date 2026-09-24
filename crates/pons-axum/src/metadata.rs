use pons_ir::{CredentialLocator, CredentialProvision, Doc, Name};

pub struct OperationMeta {
    pub(crate) id: Vec<Name>,
    pub(crate) doc: Doc,
    pub(crate) provides: Option<CredentialProvision>,
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
            expires_at: None,
            ext: Default::default(),
        });
        self
    }
}
