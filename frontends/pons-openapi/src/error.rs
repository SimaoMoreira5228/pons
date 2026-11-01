use pons_ir::ValidationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Read(String),
    NotDocument { found: String },
    MissingVersion,
    UnsupportedVersion { found: String },
    Document(String),
    Invalid(Vec<ValidationError>),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(message) => write!(
                formatter,
                "the input is not OpenAPI 3.0 or 3.1 JSON (YAML is not read): {message}"
            ),
            Self::NotDocument { found } => {
                write!(
                    formatter,
                    "the input is {found}, not an OpenAPI document object"
                )
            }
            Self::MissingVersion => formatter.write_str(
                "the OpenAPI document has no string \"openapi\" version, so it is not OpenAPI 3.0 or 3.1",
            ),
            Self::UnsupportedVersion { found } => write!(
                formatter,
                "the document declares OpenAPI {found}; this importer reads 3.0.x and 3.1.x"
            ),
            Self::Document(message) => formatter.write_str(message),
            Self::Invalid(errors) => {
                let messages = errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                write!(formatter, "the imported document is not valid:\n{messages}")
            }
        }
    }
}

impl std::error::Error for ImportError {}
