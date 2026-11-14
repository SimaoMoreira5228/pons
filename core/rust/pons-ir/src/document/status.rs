use crate::operation::{ErrorStatus, ResponseStatus};

pub(super) fn status_forbids_body(status: u16) -> bool {
    matches!(status, 100..=199 | 204 | 205 | 304)
}

fn response_answers_success(response: ResponseStatus) -> bool {
    match response {
        ResponseStatus::Exact(status) => (200..300).contains(&status),
        ResponseStatus::Class(class) => class == 2,
        ResponseStatus::Default | ResponseStatus::Unknown => true,
    }
}

pub(super) fn response_status_conflicts(response: ResponseStatus, error: &ErrorStatus) -> bool {
    match (response, error) {
        (ResponseStatus::Exact(response), ErrorStatus::Exact(error)) => response == *error,
        (ResponseStatus::Exact(response), ErrorStatus::Class(class)) => {
            response / 100 == u16::from(*class)
        }
        (ResponseStatus::Class(response), ErrorStatus::Exact(error)) => {
            u16::from(response) == *error / 100
        }
        (ResponseStatus::Class(response), ErrorStatus::Class(error)) => response == *error,
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Exact(status)) => {
            response_answers_success(ResponseStatus::Exact(*status))
        }
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Class(class)) => {
            response_answers_success(ResponseStatus::Class(*class))
        }
        (ResponseStatus::Default | ResponseStatus::Unknown, ErrorStatus::Default) => true,
        _ => false,
    }
}
