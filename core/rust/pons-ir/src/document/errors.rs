use std::collections::BTreeSet;

use crate::operation::{ErrorContent, ErrorResponse, ErrorStatus, Method, Operation};
use crate::types::Name;

use super::media::error_encoding;
use super::status::{response_status_conflicts, status_forbids_body};
use super::{Document, ValidationError};

impl Document {
    pub(super) fn validate_common_errors(&self, errors: &mut Vec<ValidationError>) {
        let mut common_error_ids = BTreeSet::new();
        let mut common_error_slots = BTreeSet::new();
        for (index, error) in self.common_errors.iter().enumerate() {
            if !common_error_slots.insert(error.status.clone()) {
                errors.push(ValidationError::InvalidValue {
                    path: "common_errors".into(),
                    message: format!("duplicate error dispatch slot {:?}", error.status),
                });
            }
            if let Some(shadowing) =
                self.common_errors
                    .iter()
                    .enumerate()
                    .find(|(other, candidate)| {
                        *other != index && candidate.status.shadows(&error.status)
                    })
            {
                errors.push(ValidationError::InvalidValue {
                    path: "common_errors".into(),
                    message: format!(
                        "error dispatch slot {:?} is shadowed: the common error {:?} already answers for it",
                        error.status, shadowing.1.status
                    ),
                });
            }
            if !common_error_ids.insert((error.status.clone(), error.name.clone())) {
                errors.push(ValidationError::InvalidValue {
                    path: "common_errors".into(),
                    message: format!("duplicate error {:?} {:?}", error.status, error.name),
                });
            }
            self.validate_error("common_errors", error, false, errors);
            self.visit_error_refs("common_errors", error, errors);
        }
    }

    pub(super) fn validate_operation_errors(
        &self,
        owner: &str,
        operation: &Operation,
        errors: &mut Vec<ValidationError>,
    ) {
        let common_error_ids: BTreeSet<(ErrorStatus, Name)> = self
            .common_errors
            .iter()
            .map(|error| (error.status.clone(), error.name.clone()))
            .collect();
        let mut operation_error_ids = BTreeSet::new();
        let mut operation_error_slots = BTreeSet::new();
        for error in &operation.errors {
            let identity = (error.status.clone(), error.name.clone());
            if !operation_error_slots.insert(error.status.clone()) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!("duplicate error dispatch slot {:?}", error.status),
                });
            } else if operation.errors_inherit
                && let Some(common) = self
                    .common_errors
                    .iter()
                    .find(|common| common.status.shadows(&error.status))
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!(
                        "error dispatch slot {:?} is inherited: the common error {:?} already answers for it",
                        error.status, common.status
                    ),
                });
            }
            if !operation_error_ids.insert(identity.clone()) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!("duplicate error {:?} {:?}", error.status, error.name),
                });
            } else if operation.errors_inherit && common_error_ids.contains(&identity) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!("error {:?} {:?} is inherited", error.status, error.name),
                });
            }
            if operation
                .responses
                .iter()
                .any(|response| response_status_conflicts(response.status, &error.status))
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.to_owned(),
                    message: format!(
                        "error status {:?} collides with a success response",
                        error.status
                    ),
                });
            }
            self.validate_error(owner, error, operation.method == Method::Head, errors);
        }
        if operation.errors_inherit {
            for error in &self.common_errors {
                if operation
                    .responses
                    .iter()
                    .any(|response| response_status_conflicts(response.status, &error.status))
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message: format!(
                            "error status {:?} collides with a success response",
                            error.status
                        ),
                    });
                }
                if operation.method == Method::Head && error.body.is_some() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.to_owned(),
                        message: format!("error {:?} cannot carry a body", error.status),
                    });
                }
            }
        }
    }

    fn validate_error(
        &self,
        owner: &str,
        error: &ErrorResponse,
        body_forbidden: bool,
        errors: &mut Vec<ValidationError>,
    ) {
        if error.name.is_empty() || error.name.iter().any(String::is_empty) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: "error name must contain non-empty name segments".into(),
            });
        }
        if let ErrorStatus::Exact(status) = error.status
            && !(100..=599).contains(&status)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("invalid error status {status}"),
            });
        }
        if let ErrorStatus::Class(class) = error.status
            && !(1..=5).contains(&class)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("invalid HTTP status class {class}xx"),
            });
        }
        if error.body.is_some()
            && (body_forbidden
                || matches!(error.status, ErrorStatus::Class(1..=3))
                || matches!(error.status, ErrorStatus::Exact(status) if status_forbids_body(status)))
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("error {:?} cannot carry a body", error.status),
            });
        }
        if (error.content == ErrorContent::Empty) == error.body.is_some() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("error content {:?} does not match its body", error.content),
            });
        }
        if error.content == ErrorContent::Empty && error.media.is_some() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: "empty errors cannot declare a media type".into(),
            });
        }
        if let Some(media) = &error.media {
            self.validate_media(
                owner,
                "error body",
                error_encoding(error.content),
                media,
                errors,
            );
        }
        let mut header_names = BTreeSet::new();
        for header in &error.headers {
            self.validate_header_name(
                owner,
                &format!("error {:?} header", error.status),
                &header.wire,
                &mut header_names,
                errors,
            );
        }
    }
}
