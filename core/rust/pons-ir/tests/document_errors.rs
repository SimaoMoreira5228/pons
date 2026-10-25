mod support;

use pons_ir::ErrorStatus;
use support::{fixture, refuses};

#[test]
fn a_common_error_class_and_the_exact_status_inside_it_both_dispatch() {
    let template = fixture();
    let with_common = |statuses: [ErrorStatus; 2]| {
        let mut document = template.clone();
        document.operations[0].errors.clear();
        let error = template.operations[0].errors[0].clone();
        for status in statuses {
            let mut common = error.clone();
            common.status = status;
            document.common_errors.push(common);
        }
        document
    };

    for dispatchable in [
        [ErrorStatus::Class(4), ErrorStatus::Exact(404)],
        [ErrorStatus::Default, ErrorStatus::Exact(418)],
        [ErrorStatus::Class(4), ErrorStatus::Class(5)],
    ] {
        with_common(dispatchable).validate().unwrap();
    }
    for (shadowing, claimed) in [
        (ErrorStatus::Class(4), ErrorStatus::Class(4)),
        (ErrorStatus::Default, ErrorStatus::Default),
    ] {
        refuses(
            &with_common([claimed.clone(), shadowing.clone()]),
            &format!(
                "error dispatch slot {claimed:?} is shadowed: the common error {shadowing:?} already answers for it"
            ),
        );
    }
}

#[test]
fn an_operation_error_outranks_a_common_error_that_answers_the_same_status() {
    let template = fixture();
    let document = |operation: ErrorStatus, common: ErrorStatus| {
        let mut document = template.clone();
        let error = &mut document.operations[0].errors[0];
        error.status = operation;
        let mut inherited = error.clone();
        inherited.status = common;
        inherited.name = vec!["common".into()];
        document.common_errors.push(inherited);
        document
    };

    document(ErrorStatus::Exact(404), ErrorStatus::Class(4))
        .validate()
        .unwrap();
    document(ErrorStatus::Exact(422), ErrorStatus::Default)
        .validate()
        .unwrap();
    refuses(
        &document(ErrorStatus::Class(4), ErrorStatus::Class(4)),
        "error dispatch slot Class(4) is inherited: the common error Class(4) already answers for it",
    );
    refuses(
        &document(ErrorStatus::Default, ErrorStatus::Default),
        "error dispatch slot Default is inherited: the common error Default already answers for it",
    );
}

#[test]
fn a_default_error_collides_only_with_a_catch_all_success() {
    let with_statuses = |response: pons_ir::ResponseStatus, error: ErrorStatus| {
        let mut document = fixture();
        let declared = &mut document.operations[0].responses[0];
        declared.status = response;
        if matches!(response, pons_ir::ResponseStatus::Exact(status) if matches!(status, 100..=199 | 204 | 205 | 304))
        {
            declared.content = pons_ir::ContentKind::Empty;
            declared.body = None;
        }
        document.operations[0].errors[0].status = error;
        document
    };
    let exact = |status| pons_ir::ResponseStatus::Exact(status);
    let collides = "collides with a success response";

    refuses(
        &with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Default),
        &format!("error status Default {collides}"),
    );
    refuses(
        &with_statuses(pons_ir::ResponseStatus::Unknown, ErrorStatus::Default),
        &format!("error status Default {collides}"),
    );
    refuses(
        &with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Exact(200)),
        &format!("error status Exact(200) {collides}"),
    );
    refuses(
        &with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Class(2)),
        &format!("error status Class(2) {collides}"),
    );

    for document in [
        with_statuses(exact(200), ErrorStatus::Default),
        with_statuses(exact(204), ErrorStatus::Default),
        with_statuses(exact(302), ErrorStatus::Default),
        with_statuses(exact(404), ErrorStatus::Default),
        with_statuses(pons_ir::ResponseStatus::Class(4), ErrorStatus::Default),
        with_statuses(pons_ir::ResponseStatus::Class(2), ErrorStatus::Default),
        with_statuses(pons_ir::ResponseStatus::Default, ErrorStatus::Exact(404)),
        with_statuses(exact(404), ErrorStatus::Class(5)),
    ] {
        document.validate().unwrap();
    }
}

#[test]
fn an_empty_error_name_is_refused() {
    for name in [vec![], vec!["bad".into(), String::new()]] {
        let mut document = fixture();
        document.operations[0].errors[0].name = name;
        refuses(&document, "error name must contain non-empty name segments");
    }
    fixture().validate().unwrap();
}
