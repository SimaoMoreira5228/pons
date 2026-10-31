use serde_json::{Map, Value};

#[derive(Clone, Copy)]
pub(crate) enum Section {
    Parameters,
    RequestBodies,
    Responses,
}

impl Section {
    fn prefix(self) -> &'static str {
        match self {
            Self::Parameters => "#/components/parameters/",
            Self::RequestBodies => "#/components/requestBodies/",
            Self::Responses => "#/components/responses/",
        }
    }
}

pub(crate) struct Components {
    pub(crate) parameters: Map<String, Value>,
    pub(crate) request_bodies: Map<String, Value>,
    pub(crate) responses: Map<String, Value>,
}

impl Components {
    pub(crate) fn new(root: &Map<String, Value>) -> Self {
        let components = root.get("components").and_then(Value::as_object);
        Self {
            parameters: section(components, "parameters"),
            request_bodies: section(components, "requestBodies"),
            responses: section(components, "responses"),
        }
    }

    pub(crate) fn resolve<'a>(&'a self, value: &'a Value, section: Section) -> Option<&'a Value> {
        let declared = match section {
            Section::Parameters => &self.parameters,
            Section::RequestBodies => &self.request_bodies,
            Section::Responses => &self.responses,
        };
        let mut current = value;
        for _ in 0..8 {
            let Some(reference) = current.get("$ref").and_then(Value::as_str) else {
                return Some(current);
            };
            let name = reference.strip_prefix(section.prefix())?;
            current = declared.get(name)?;
        }
        None
    }
}

fn section(components: Option<&Map<String, Value>>, name: &str) -> Map<String, Value> {
    components
        .and_then(|components| components.get(name))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}
