mod support;

use axum::response::IntoResponse;
use pons_axum::routing::{get_with, head_with, post};
use pons_axum::{Json, NoContent, OperationMeta, Router, WithErrorHeader};
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{ErrorContent, ErrorResponse, ErrorStatus, Operation};
use serde::Serialize;
use support::validation::*;
use support::*;

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum CommonRejection {
    #[allow(dead_code)]
    #[pons(status = 422)]
    Rejected,
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum RangeRejection {
    #[allow(dead_code)]
    #[pons(status = "4xx")]
    Rejected,
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum DefaultRejection {
    #[allow(dead_code)]
    #[pons(status = "default")]
    Rejected,
}

struct RangeRejectionReport;

impl ErrorSpec for RangeRejectionReport {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        pons_axum::ViolationReport::errors(cx)
            .into_iter()
            .map(|mut error| {
                error.status = ErrorStatus::Class(4);
                error.name = vec!["range".into()];
                error
            })
            .collect()
    }
}

struct SilentRejectionReport;

impl ErrorSpec for SilentRejectionReport {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        pons_axum::ViolationReport::errors(cx)
            .into_iter()
            .map(|mut error| {
                error.retryable = None;
                error.name = vec!["silent".into()];
                error
            })
            .collect()
    }
}

async fn ping() -> NoContent {
    NoContent
}

fn inherited_router() -> Router<()> {
    Router::new(info())
        .common_error::<CommonRejection>()
        .route("/inherited", post(create_open_user))
}

struct ProblemRejection;

impl ErrorSpec for ProblemRejection {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        pons_axum::ViolationReport::errors(cx)
            .into_iter()
            .map(|mut error| {
                error.content = ErrorContent::ProblemJson;
                error
            })
            .collect()
    }
}

fn assert_no_collision(operation: &Operation) {
    assert!(
        !operation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-E102"),
        "{:?} must report no disagreement with the common error",
        operation.diagnostics
    );
}

#[test]
fn a_common_error_that_adds_a_header_the_runtime_never_sends_does_not_own_the_slot() {
    fn router() -> Router<()> {
        Router::new(info())
            .common_error::<WithErrorHeader<RequestId, pons_axum::ViolationReport>>()
            .route("/headed", post(create_open_user))
    }
    let errors = refused(router());
    assert!(
        errors.contains("PONS-E102") && errors.contains("response headers"),
        "{errors} must name the header the common error adds to the rejection"
    );

    let document = router().document();
    let headed = operation(&document, "headed");
    assert_eq!(
        headed.errors.len(),
        1,
        "{:?} keeps the rejection the runtime can serve",
        headed.errors
    );
    assert!(
        headed.errors[0].headers.is_empty(),
        "the report the runtime sends carries no {:?}",
        headed.errors[0].headers
    );
}

#[test]
fn a_common_error_that_answers_the_rejection_as_problem_json_does_not_own_the_slot() {
    let errors = refused(
        Router::new(info())
            .common_error::<ProblemRejection>()
            .route("/problem", post(create_open_user)),
    );
    assert!(
        errors.contains("PONS-E102") && errors.contains("content"),
        "{errors} must name the content the common error answers the rejection with"
    );
}

#[test]
fn a_foreign_common_error_at_the_rejection_status_is_refused_instead_of_published() {
    let errors = refused(inherited_router());
    assert!(
        errors.contains("PONS-E102") && errors.contains("422") && errors.contains("[\"rejected\"]"),
        "{errors} must name the collision and the common error that occupies it"
    );
    assert!(
        refused_build(inherited_router()).contains("PONS-E102"),
        "a router whose document does not validate must refuse to be built"
    );
}

#[test]
fn an_inherited_violation_report_leaves_the_slot_to_the_common_error() {
    let document = Router::<()>::new(info())
        .common_error::<pons_axum::ViolationReport>()
        .route("/inherited", post(create_open_user))
        .into_document()
        .unwrap();
    assert_eq!(document.common_errors.len(), 1);
    assert_eq!(document.common_errors[0].status, ErrorStatus::Exact(422));
    let inherited = operation(&document, "inherited");
    assert!(
        inherited.errors.is_empty(),
        "{:?} must leave the inherited slot to the common report",
        inherited.errors
    );
    assert_no_collision(inherited);
    assert!(document.diagnostics.is_empty());
}

#[test]
fn a_head_operation_does_not_inherit_a_common_error_that_carries_a_body() {
    fn router() -> Router<()> {
        Router::new(info())
            .common_error::<CommonRejection>()
            .route("/head", head_with(ping, OperationMeta::new("head.ping")))
            .route("/get", get_with(ping, OperationMeta::new("get.ping")))
    }
    let document = router()
        .into_document()
        .expect("a common error that carries a body must not unpublish every HEAD route");
    let head = operation(&document, "head");
    assert!(
        !head.errors_inherit,
        "a HEAD response carries no body, so it inherits no error that spells one"
    );
    assert!(
        head.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PONS-W101" && diagnostic.message.contains("common error")
        }),
        "{:?} must say which common error it does not inherit",
        head.diagnostics
    );
    let get = operation(&document, "get");
    assert!(
        get.errors_inherit,
        "a GET answers the common error that carries a body"
    );
    assert!(
        !get.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("common error")),
        "{:?} inherits the common error, so it cannot say it does not",
        get.diagnostics
    );
}

#[test]
fn a_common_error_range_that_covers_the_rejection_is_diagnosed_before_it_is_published() {
    for (router, covering) in [
        (
            Router::<()>::new(info())
                .common_error::<RangeRejection>()
                .route("/ranged", post(create_open_user)),
            "Class(4)",
        ),
        (
            Router::<()>::new(info())
                .common_error::<DefaultRejection>()
                .route("/ranged", post(create_open_user)),
            "Default",
        ),
    ] {
        let document = router.document();
        let ranged = operation(&document, "ranged");
        assert!(
            ranged.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "PONS-E102"
                    && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
                    && diagnostic.message.contains(covering)
            }),
            "{:?} must name the common error {covering} that answers the rejection",
            ranged.diagnostics
        );

        let errors = refused_build(router);
        assert!(
            errors.contains("PONS-E102") && errors.contains("a different body and retryable"),
            "{errors} must refuse to install a document whose rejection slot another error answers"
        );
    }
}

#[test]
fn a_common_error_range_that_answers_the_rejection_the_same_way_leaves_the_slot_to_it() {
    let router = Router::<()>::new(info())
        .common_error::<RangeRejectionReport>()
        .route("/ranged", post(create_open_user));
    let document = router.into_document().unwrap();
    let ranged = operation(&document, "ranged");
    assert!(
        ranged.errors.is_empty(),
        "{:?} must leave the inherited slot to the common error",
        ranged.errors
    );
    assert_no_collision(ranged);
    assert_eq!(document.common_errors[0].status, ErrorStatus::Class(4));
}

#[test]
fn a_common_error_that_spells_the_media_type_the_rejection_answers_with_owns_the_slot() {
    struct LoudRejectionReport;

    impl ErrorSpec for LoudRejectionReport {
        fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
            pons_axum::ViolationReport::errors(cx)
                .into_iter()
                .map(|mut error| {
                    error.media = Some(pons_ir::MediaDescriptor::from_declared("application/json"));
                    error.name = vec!["loud".into()];
                    error
                })
                .collect()
        }
    }

    let document = Router::<()>::new(info())
        .common_error::<LoudRejectionReport>()
        .route("/loud", post(create_open_user))
        .into_document()
        .expect("the media type the rejection answers with is not a disagreement");
    assert_eq!(
        document.common_errors[0]
            .media
            .as_ref()
            .map(|media| media.to_string())
            .as_deref(),
        Some("application/json"),
        "the common error spells what the rejection's content kind already answers"
    );
    let loud = operation(&document, "loud");
    assert!(
        loud.errors.is_empty(),
        "{:?} must leave the slot to the common error that answers the same media type",
        loud.errors
    );
    assert_no_collision(loud);
}

#[test]
fn a_common_error_that_leaves_retryable_out_does_not_own_the_slot_the_client_reads() {
    fn router() -> Router<()> {
        Router::<()>::new(info())
            .common_error::<SilentRejectionReport>()
            .route("/silent", post(create_open_user))
    }
    let document = router().document();
    assert_eq!(
        declared_rejection(&document, "silent")
            .expect("/silent declares the rejection the runtime can serve")
            .retryable,
        Some(false),
        "the rejection the client reads is never retryable"
    );
    assert_eq!(document.common_errors[0].retryable, None);
    let diagnostic = operation(&document, "silent")
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-E102")
        .expect("a common error that never says whether it is retryable cannot own the slot");
    assert!(
        diagnostic.message.contains("retryable"),
        "{} must name the field the client branches on",
        diagnostic.message
    );

    let errors = refused(router());
    assert!(
        errors.contains("PONS-E102") && errors.contains("retryable"),
        "{errors} must refuse a document whose inherited slot no longer says a request is not retryable"
    );
}

#[test]
fn a_common_error_that_leaves_retryable_out_as_the_rejection_does_owns_the_slot() {
    struct SilentRejection(pons_axum::ViolationReport);

    impl ErrorSpec for SilentRejection {
        fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
            pons_axum::ViolationReport::errors(cx)
                .into_iter()
                .map(|mut error| {
                    error.retryable = None;
                    error
                })
                .collect()
        }
    }

    impl IntoResponse for SilentRejection {
        fn into_response(self) -> axum::response::Response {
            self.0.into_response()
        }
    }

    async fn declares_its_own_silent_rejection(
        Json(_body): Json<OpenUser>,
    ) -> Result<axum::Json<User>, SilentRejection> {
        Err(SilentRejection(pons_axum::ViolationReport {
            violations: Vec::new(),
        }))
    }

    let document = Router::<()>::new(info())
        .common_error::<SilentRejectionReport>()
        .route("/silent", post(declares_its_own_silent_rejection))
        .into_document()
        .expect("a value both errors leave out is not a disagreement");
    assert_eq!(document.common_errors[0].retryable, None);
    let silent = operation(&document, "silent");
    assert!(
        silent.errors.is_empty(),
        "{:?} must leave the slot to the common error that leaves the field out too",
        silent.errors
    );
    assert!(
        !silent
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-E102"),
        "{:?} must not report a field neither error spells",
        silent.diagnostics
    );
}

#[test]
fn a_common_error_that_answers_the_rejection_as_another_media_type_does_not_own_the_slot() {
    struct VendorRejectionReport;

    impl ErrorSpec for VendorRejectionReport {
        fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
            pons_axum::ViolationReport::errors(cx)
                .into_iter()
                .map(|mut error| {
                    error.media = Some(pons_ir::MediaDescriptor::from_declared(
                        "application/problem+json",
                    ));
                    error.name = vec!["vendor".into()];
                    error
                })
                .collect()
        }
    }

    fn router() -> Router<()> {
        Router::<()>::new(info())
            .common_error::<VendorRejectionReport>()
            .route("/vendored", post(create_open_user))
    }
    let document = router().document();
    let rejection = declared_rejection(&document, "vendored")
        .expect("/vendored declares the rejection the runtime can serve");
    assert_eq!(
        rejection.media, None,
        "the runtime answers the rejection as the content kind spells it, application/json"
    );
    assert_eq!(
        document.common_errors[0]
            .media
            .as_ref()
            .map(|media| media.to_string())
            .as_deref(),
        Some("application/problem+json")
    );
    let diagnostic = operation(&document, "vendored")
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-E102")
        .expect(
            "a common error that answers the rejection as another media type must be diagnosed",
        );
    assert!(
        diagnostic.message.contains("media type"),
        "{} must name the field the two errors spell differently",
        diagnostic.message
    );

    let errors = refused(router());
    assert!(
        errors.contains("PONS-E102") && errors.contains("media type"),
        "{errors} must refuse a document whose inherited slot answers a media type the runtime never sends"
    );
}

#[test]
fn a_common_error_that_answers_the_rejection_differently_names_both_error_types() {
    struct ProblemAndVendor;

    impl ErrorSpec for ProblemAndVendor {
        fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
            CommonRejection::errors(cx)
                .into_iter()
                .map(|mut error| {
                    error.name = vec!["problem".into()];
                    error.content = ErrorContent::ProblemJson;
                    error.ext.insert("vendor".into(), "acme".into());
                    error
                })
                .collect()
        }
    }
    let document = Router::<()>::new(info())
        .common_error::<ProblemAndVendor>()
        .route("/problem", post(create_open_user))
        .document();
    let problem = operation(&document, "problem");
    let diagnostic = problem
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-E102")
        .expect("a foreign common error at the rejection must be diagnosed");
    assert!(
        diagnostic.message.contains("content") && diagnostic.message.contains("extensions"),
        "{} must report every field the two errors spell differently",
        diagnostic.message
    );
    assert!(
        diagnostic
            .message
            .contains("validation_common_errors::CommonRejection#output")
            && diagnostic
                .message
                .contains("pons_axum::error::ViolationReport#output"),
        "{} must name the two error type ids",
        diagnostic.message
    );
}
