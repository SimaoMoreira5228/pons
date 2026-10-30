use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pagination {
    pub request: PaginationRequest,
    pub items: JsonPointer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<JsonPointer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<PageSize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaginationRequest {
    Cursor { field: String },
    Offset { field: String },
}

impl PaginationRequest {
    pub fn field(&self) -> &str {
        match self {
            Self::Cursor { field } | Self::Offset { field } => field,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageSize {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct JsonPointer(String);

impl JsonPointer {
    pub fn new(pointer: impl Into<String>) -> Self {
        Self(pointer.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.0.is_empty() && !self.0.starts_with('/') {
            return Err("a JSON pointer must be empty or start with '/'".into());
        }
        let mut characters = self.0.chars();
        while let Some(character) = characters.next() {
            if character == '~' {
                let Some(next) = characters.next() else {
                    return Err("a JSON pointer has an incomplete escape".into());
                };
                if next != '0' && next != '1' {
                    return Err("a JSON pointer has an invalid escape".into());
                }
            }
        }
        Ok(())
    }
}

impl Pagination {
    pub fn validate(&self) -> Result<(), String> {
        if self.request.field().is_empty() {
            return Err("pagination request field cannot be empty".into());
        }
        if matches!(self.request, PaginationRequest::Cursor { .. }) && self.next.is_none() {
            return Err(
                "a cursor request needs a next pointer, because the iterator cannot advance past the first page without one"
                    .into(),
            );
        }
        self.items
            .validate()
            .map_err(|message| format!("pagination item locator: {message}"))?;
        if matches!(self.request, PaginationRequest::Offset { .. }) && self.next.is_some() {
            return Err(
                "an offset request cannot declare a next pointer, because the client advances the offset itself"
                    .into(),
            );
        }
        if let Some(next) = &self.next {
            next.validate()
                .map_err(|message| format!("pagination next locator: {message}"))?;
        }
        if let Some(page_size) = &self.page_size {
            if page_size.field.is_empty() {
                return Err("pagination page size field cannot be empty".into());
            }
            if page_size.default == Some(0) {
                return Err("pagination page size default must be positive".into());
            }
            if page_size.maximum == Some(0) {
                return Err("pagination page size maximum must be positive".into());
            }
            if let (Some(default), Some(maximum)) = (page_size.default, page_size.maximum)
                && default > maximum
            {
                return Err(format!(
                    "pagination page size default {default} exceeds its maximum {maximum}"
                ));
            }
        }
        Ok(())
    }
}
