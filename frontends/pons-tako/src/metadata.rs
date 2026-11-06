use pons_ir::{Doc, Name};

pub struct OperationMeta {
    pub(crate) id: Vec<Name>,
    pub(crate) doc: Doc,
}

impl OperationMeta {
    pub fn new(id: impl Into<String>) -> Self {
        let id = id
            .into()
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

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.doc.tags.push(tag.into());
        self
    }

    pub fn deprecated(mut self, deprecated: bool) -> Self {
        self.doc.deprecated = Some(deprecated);
        self
    }
}
