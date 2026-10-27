use pons_ir::{Document, ErrorStatus, ResponseStatus};

#[test]
fn validation_rejects_duplicate_error_identities() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut duplicate = error.clone();
    duplicate.retryable = Some(true);
    document.common_errors.push(error);
    document.common_errors.push(duplicate);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut duplicate = error.clone();
    duplicate.retryable = Some(true);
    document.operations[0].errors.push(duplicate);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    document.common_errors.push(error);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("is inherited"))
    );

    let mut document = base;
    let error = document.operations[0].errors[0].clone();
    document.common_errors.push(error);
    document.operations[0].errors_inherit = false;
    assert!(document.validate().is_ok());
}

#[test]
fn validation_rejects_duplicate_error_dispatch_slots() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut other = error.clone();
    other.name = vec!["other".into()];
    document.common_errors.push(error);
    document.common_errors.push(other);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error dispatch slot"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    let mut other = error.clone();
    other.name = vec!["other".into()];
    document.operations[0].errors.push(other);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error dispatch slot"))
    );

    let mut document = base.clone();
    let error = document.operations[0].errors[0].clone();
    document.common_errors.push(error);
    assert!(
        document
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("dispatch slot")
                && error.to_string().contains("inherited"))
    );

    let mut document = base;
    let mut exact = document.operations[0].errors[0].clone();
    exact.status = ErrorStatus::Exact(400);
    exact.name = vec!["exact".into()];
    document.operations[0].errors.push(exact);
    assert!(document.validate().is_ok());
}

#[test]
fn validation_refuses_an_operation_error_a_common_error_already_answers_for() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let with_common = |operation: ErrorStatus, common: ErrorStatus| {
        let mut document = base.clone();
        let rejected = &mut document.operations[0].errors[0];
        rejected.status = operation;
        rejected.name = vec!["request".into(), "rejected".into()];
        let mut inherited = rejected.clone();
        inherited.status = common;
        document.common_errors.push(inherited);
        document
    };
    let inherited = |document: &Document, shadowing: &str| {
        document.validate().unwrap_err().iter().any(|error| {
            let error = error.to_string();
            error.contains("error dispatch slot")
                && error.contains("is inherited")
                && error.contains(shadowing)
        })
    };

    assert!(inherited(
        &with_common(ErrorStatus::Class(4), ErrorStatus::Class(4)),
        "Class(4)"
    ));
    assert!(inherited(
        &with_common(ErrorStatus::Default, ErrorStatus::Default),
        "Default"
    ));

    with_common(ErrorStatus::Exact(404), ErrorStatus::Class(4))
        .validate()
        .unwrap();
    with_common(ErrorStatus::Exact(422), ErrorStatus::Default)
        .validate()
        .unwrap();
    with_common(ErrorStatus::Exact(422), ErrorStatus::Class(5))
        .validate()
        .unwrap();
    assert!(
        with_common(ErrorStatus::Exact(422), ErrorStatus::Exact(422))
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("is inherited"))
    );
}

#[test]
fn validation_refuses_two_common_errors_that_shadow_one_dispatch_slot() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let with_common = |first: ErrorStatus, second: ErrorStatus| {
        let mut document = base.clone();
        let template = document.operations[0].errors[0].clone();
        document.operations[0].errors.clear();
        for status in [first, second] {
            let mut common = template.clone();
            common.status = status;
            document.common_errors.push(common);
        }
        document
    };
    let shadowed = |document: &Document, shadowing: &str| {
        document.validate().unwrap_err().iter().any(|error| {
            let error = error.to_string();
            error.contains("error dispatch slot")
                && error.contains("is shadowed")
                && error.contains(shadowing)
        })
    };

    assert!(shadowed(
        &with_common(ErrorStatus::Class(4), ErrorStatus::Class(4)),
        "Class(4)"
    ));
    assert!(shadowed(
        &with_common(ErrorStatus::Default, ErrorStatus::Default),
        "Default"
    ));

    assert!(
        with_common(ErrorStatus::Exact(422), ErrorStatus::Exact(422))
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.to_string().contains("duplicate error dispatch slot"))
    );
    for dispatchable in [
        with_common(ErrorStatus::Exact(404), ErrorStatus::Class(4)),
        with_common(ErrorStatus::Exact(422), ErrorStatus::Default),
        with_common(ErrorStatus::Exact(422), ErrorStatus::Class(5)),
    ] {
        dispatchable.validate().unwrap();
    }
}

#[test]
fn an_error_status_covers_the_statuses_a_runtime_can_answer_it_for() {
    assert!(ErrorStatus::Class(4).covers(&ErrorStatus::Exact(422)));
    assert!(ErrorStatus::Class(4).covers(&ErrorStatus::Class(4)));
    assert!(ErrorStatus::Default.covers(&ErrorStatus::Exact(418)));
    assert!(ErrorStatus::Default.covers(&ErrorStatus::Default));
    assert!(ErrorStatus::Exact(422).covers(&ErrorStatus::Exact(422)));
    assert!(!ErrorStatus::Exact(422).covers(&ErrorStatus::Class(4)));
    assert!(!ErrorStatus::Class(4).covers(&ErrorStatus::Exact(503)));
    assert!(!ErrorStatus::Class(4).covers(&ErrorStatus::Default));
}

#[test]
fn an_error_status_shadows_only_the_slots_it_answers_for_the_same() {
    assert!(ErrorStatus::Class(4).shadows(&ErrorStatus::Class(4)));
    assert!(ErrorStatus::Default.shadows(&ErrorStatus::Default));
    assert!(!ErrorStatus::Class(4).shadows(&ErrorStatus::Exact(404)));
    assert!(!ErrorStatus::Default.shadows(&ErrorStatus::Class(4)));
    assert!(!ErrorStatus::Default.shadows(&ErrorStatus::Exact(404)));
    assert!(!ErrorStatus::Exact(404).shadows(&ErrorStatus::Class(4)));
    assert!(!ErrorStatus::Exact(404).shadows(&ErrorStatus::Default));
    assert!(!ErrorStatus::Exact(404).shadows(&ErrorStatus::Exact(404)));
    assert!(!ErrorStatus::Class(4).shadows(&ErrorStatus::Class(5)));
}

#[test]
fn validation_rejects_exact_error_response_collisions() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let mut document = base.clone();
    document.operations[0].errors[0].status = ErrorStatus::Exact(200);
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("collides with a success response")
    }));

    let mut document = base.clone();
    let mut common = document.operations[0].errors[0].clone();
    common.status = ErrorStatus::Exact(200);
    document.common_errors.push(common);
    assert!(document.validate().unwrap_err().iter().any(|error| {
        error
            .to_string()
            .contains("collides with a success response")
    }));

    let mut document = base;
    document.operations[0].responses[0].status = ResponseStatus::Exact(400);
    document.operations[0].errors[0].status = ErrorStatus::Class(5);
    assert!(document.validate().is_ok());
}

#[test]
fn validation_rejects_overlapping_response_error_status_dispatch() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let has_collision = |document: &Document| {
        document.validate().unwrap_err().iter().any(|error| {
            error
                .to_string()
                .contains("collides with a success response")
        })
    };

    let mut document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(4);
    document.operations[0].errors[0].status = ErrorStatus::Class(4);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(4);
    document.operations[0].errors[0].status = ErrorStatus::Exact(404);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Unknown;
    document.operations[0].errors[0].status = ErrorStatus::Class(2);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Exact(201);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Class(4);
    let mut common = document.operations[0].errors[0].clone();
    common.name = vec!["common".into()];
    document.common_errors.push(common);
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Unknown;
    document.operations[0].errors[0].status = ErrorStatus::Default;
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Default;
    assert!(has_collision(&document));

    document = base.clone();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Class(5);
    document.validate().unwrap();

    let mut disjoint = base;
    disjoint.operations[0].responses[0].status = ResponseStatus::Class(2);
    disjoint.operations[0].errors[0].status = ErrorStatus::Class(4);
    disjoint.validate().unwrap();
}

#[test]
fn validation_rejects_two_catch_all_success_responses() {
    let base: Document =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    let with_catch_alls = |first: ResponseStatus, second: ResponseStatus| {
        let mut document = base.clone();
        document.operations[0].responses[0].status = first;
        let mut other = document.operations[0].responses[0].clone();
        other.status = second;
        document.operations[0].responses.push(other);
        document
    };

    for document in [
        with_catch_alls(ResponseStatus::Default, ResponseStatus::Unknown),
        with_catch_alls(ResponseStatus::Unknown, ResponseStatus::Default),
    ] {
        assert!(document.validate().unwrap_err().iter().any(|error| {
            error
                .to_string()
                .contains("duplicate response status default")
        }));
    }

    for status in [ResponseStatus::Default, ResponseStatus::Unknown] {
        let mut document = base.clone();
        document.operations[0].responses[0].status = status;
        document.validate().unwrap();
    }
}
