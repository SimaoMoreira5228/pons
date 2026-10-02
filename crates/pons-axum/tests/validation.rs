use axum::body::Body;
use axum::http::StatusCode;
use pons_axum::extract::Multipart;
use pons_axum::http::Request;
use pons_axum::routing::{get, post};
use pons_axum::{Header, HeaderSpec, Info, Json, Router};
use pons_core::{Schema, TypeCx};
use pons_ir::{
    ContentKind, Direction, ErrorContent, ErrorResponse, ErrorStatus, Field, InlineType,
    IntegerWire, ObjectOpenness, Operation, PathSegment, Presence, TypeDef, TypeKind, TypeRef,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use tower_service::Service;

#[derive(Deserialize)]
#[serde(transparent)]
struct Slug(String);

impl Schema for Slug {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: Some(1),
            max_length: Some(16),
            pattern: Some("^[a-z][a-z0-9-]*$".into()),
        })
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct Retries(u8);

impl Schema for Retries {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Int {
            bits: 8,
            signed: false,
            wire: IntegerWire::Number,
            minimum: Some(json!(1)),
            maximum: Some(json!(5)),
            multiple_of: Some(json!(1)),
        })
    }
}

impl std::str::FromStr for Retries {
    type Err = <u8 as std::str::FromStr>::Err;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse::<u8>().map(Self)
    }
}

struct RetriesHeader;

impl HeaderSpec for RetriesHeader {
    type Value = Retries;
    const NAME: &'static str = "X-Retries";
}

struct DebugFlag;

impl HeaderSpec for DebugFlag {
    type Value = bool;
    const NAME: &'static str = "X-Debug";
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct CreateUser {
    slug: Slug,
    retries: Retries,
    tags: BTreeSet<String>,
}

#[derive(Deserialize, Schema)]
struct OpenUser {
    slug: Slug,
}

#[derive(Deserialize)]
struct OpenRequest {
    slug: Slug,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl Schema for OpenRequest {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        cx.named::<Self>(
            vec!["open".into(), "request".into()],
            direction,
            None,
            |cx| TypeKind::Struct {
                fields: vec![Field {
                    wire: "slug".into(),
                    ident: vec!["slug".into()],
                    ty: Slug::schema(cx, direction),
                    presence: Presence::Required,
                    default: Default::default(),
                    access: None,
                    doc: None,
                    diagnostics: Vec::new(),
                    ext: Default::default(),
                }],
                flattened: Vec::new(),
                openness: ObjectOpenness::PreserveTyped {
                    value: <BTreeMap<String, Retries> as Schema>::schema(cx, direction),
                },
            },
        )
    }
}

#[derive(Deserialize)]
struct Unpublished;

impl Schema for Unpublished {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: "unpublished::Type#input".into(),
            variant: None,
        })
    }
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Search {
    slug: Slug,
    #[serde(default)]
    limit: Option<Retries>,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct UserPath {
    slug: Slug,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct SlugForm {
    slug: Slug,
}

#[derive(Serialize, Schema)]
struct User {
    id: u64,
}

#[derive(Serialize, Schema)]
struct UserEvent {
    id: u64,
}

impl pons_axum::SseEvent for UserEvent {
    const NAME: &'static str = "user";
}

async fn create_user(Json(input): Json<CreateUser>) -> axum::Json<User> {
    let _ = (input.slug, input.retries, input.tags);
    axum::Json(User { id: 1 })
}

async fn create_open_user(Json(input): Json<OpenUser>) -> axum::Json<User> {
    let _ = input.slug;
    axum::Json(User { id: 1 })
}

async fn create_typed_open_request(Json(input): Json<OpenRequest>) -> axum::Json<User> {
    let _ = (input.slug, input.extra);
    axum::Json(User { id: 1 })
}

async fn search(pons_axum::Query(query): pons_axum::Query<Search>) -> axum::Json<User> {
    let _ = (query.slug.0, query.limit.map(|limit| limit.0));
    axum::Json(User { id: 1 })
}

async fn get_user(pons_axum::Path(path): pons_axum::Path<UserPath>) -> axum::Json<User> {
    let _ = path.slug.0;
    axum::Json(User { id: 1 })
}

async fn create_form_user(pons_axum::Form(form): pons_axum::Form<SlugForm>) -> axum::Json<User> {
    let _ = form.slug.0;
    axum::Json(User { id: 1 })
}

async fn retried(Header(retries): Header<RetriesHeader>) -> axum::Json<User> {
    let _ = retries.0;
    axum::Json(User { id: 1 })
}

async fn flagged(Header(flag): Header<DebugFlag>) -> axum::Json<User> {
    let _ = flag;
    axum::Json(User { id: 1 })
}

async fn note(_note: String) -> axum::Json<User> {
    axum::Json(User { id: 1 })
}

async fn upload(_upload: Multipart) -> axum::Json<User> {
    axum::Json(User { id: 1 })
}

async fn unpublished(Json(_body): Json<Unpublished>) -> axum::Json<User> {
    axum::Json(User { id: 1 })
}

async fn user_events(
    pons_axum::Query(query): pons_axum::Query<Search>,
) -> pons_axum::Sse<UserEvent, impl futures_util::Stream<Item = UserEvent> + Send> {
    let _ = (query.slug.0, query.limit.map(|limit| limit.0));
    pons_axum::Sse::new(futures_util::stream::iter([UserEvent { id: 1 }]))
}

fn info() -> Info {
    Info {
        title: "Validation".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: Default::default(),
    }
}

fn router() -> Router<()> {
    Router::new(info())
        .route("/users", post(create_user))
        .route("/open-users", post(create_open_user))
        .route("/typed-open", post(create_typed_open_request))
        .route("/search", get(search))
        .route("/users/{slug}", get(get_user))
        .route("/form", post(create_form_user))
        .route("/retried", get(retried))
        .route("/flagged", get(flagged))
        .route("/note", post(note))
        .route("/upload", post(upload))
        .route("/events", get(user_events))
}

fn unpublished_router() -> Router<()> {
    Router::new(info()).route("/unpublished", post(unpublished))
}

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);

    impl std::task::Wake for ThreadWaker {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = std::task::Waker::from(std::sync::Arc::new(ThreadWaker(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

fn request(path: &str, content_type: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", content_type)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn json_request(path: &str, body: &str) -> Request<Body> {
    request(path, "application/json", body)
}

fn get_request(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

fn header_request(path: &str, name: &str, value: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(name, value)
        .body(Body::empty())
        .unwrap()
}

fn send(app: &mut axum::Router<()>, request: Request<Body>) -> (StatusCode, String) {
    let response = block_on(app.call(request)).unwrap();
    let status = response.status();
    let bytes = block_on(axum::body::to_bytes(response.into_body(), usize::MAX)).unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn accepted(app: &mut axum::Router<()>, request: Request<Body>) {
    let (status, body) = send(app, request);
    assert_eq!(status, StatusCode::OK, "{body}");
}

fn rejected(app: &mut axum::Router<()>, request: Request<Body>, expected: &[&str]) {
    let (status, body) = send(app, request);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    let found: Vec<String> = report["violations"]
        .as_array()
        .unwrap_or_else(|| panic!("{body} is not a violation report"))
        .iter()
        .map(|violation| {
            format!(
                "{}: {}",
                violation["path"].as_str().unwrap(),
                violation["reason"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(found, expected, "{body}");
}

fn operation<'a>(document: &'a pons_ir::Document, literal: &str) -> &'a Operation {
    document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == literal))
        })
        .unwrap()
}

fn declared_rejection(document: &pons_ir::Document, literal: &str) -> Option<ErrorResponse> {
    operation(document, literal)
        .errors
        .iter()
        .find(|error| error.name == ["request", "rejected"])
        .cloned()
}

fn body_type<'a>(document: &'a pons_ir::Document, id: &str) -> &'a TypeDef {
    document
        .types
        .get(id)
        .unwrap_or_else(|| panic!("{id} is unpublished"))
}

fn openness(document: &pons_ir::Document, id: &str) -> ObjectOpenness {
    let TypeKind::Struct { openness, .. } = &body_type(document, id).definition else {
        panic!("{id} must publish a struct");
    };
    openness.clone()
}

#[test]
fn a_body_string_that_breaks_the_published_pattern_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        json_request(
            "/users",
            r#"{"slug":"Ada Lovelace","retries":2,"tags":["x"]}"#,
        ),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_body_number_outside_its_published_bounds_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","retries":9,"tags":["x"]}"#),
        &["/retries: integer is above its maximum of 5"],
    );
}

#[test]
fn a_body_missing_a_required_field_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","tags":["x"]}"#),
        &["/retries: missing required field retries"],
    );
}

#[test]
fn a_body_that_repeats_an_item_of_a_unique_array_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","retries":2,"tags":["x","x"]}"#),
        &["/tags: array item 1 repeats item 0"],
    );
}

#[test]
fn an_unknown_property_is_rejected_against_deny_unknown_fields() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        json_request(
            "/users",
            r#"{"slug":"ada","retries":2,"tags":["x"],"nickname":"ada"}"#,
        ),
        &["/nickname: unknown property nickname"],
    );
}

#[test]
fn a_body_that_is_not_json_is_rejected_by_the_same_declared_response() {
    let mut app = router().into_axum();
    let (status, body) = send(&mut app, json_request("/users", "slug=ada"));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    let reported = report["violations"].as_array().unwrap();
    assert_eq!(reported.len(), 1, "{body} must report one unreadable body");
    assert_eq!(reported[0]["path"], "");
    assert!(
        reported[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("expected a JSON document"),
        "{body} must name the JSON document it could not read"
    );
}

#[test]
fn a_query_outside_its_published_bounds_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        get_request("/search?slug=ada&limit=9"),
        &["/limit: integer is above its maximum of 5"],
    );
    rejected(
        &mut app,
        get_request("/search?limit=2"),
        &["/slug: missing required field slug"],
    );
}

#[test]
fn a_path_parameter_outside_its_published_pattern_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        get_request("/users/Ada%20Lovelace"),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_form_field_outside_its_published_pattern_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        request(
            "/form",
            "application/x-www-form-urlencoded",
            "slug=Ada%20Lovelace",
        ),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_header_value_outside_its_published_bounds_is_rejected() {
    let mut app = router().into_axum();
    rejected(
        &mut app,
        header_request("/retried", "x-retries", "9"),
        &[": integer is above its maximum of 5"],
    );
}

#[test]
fn a_header_that_is_not_the_text_token_its_type_reads_is_rejected() {
    let mut app = router().into_axum();
    accepted(&mut app, header_request("/flagged", "x-debug", "true"));
    rejected(
        &mut app,
        header_request("/flagged", "x-debug", "1"),
        &[": expected a boolean, found a string"],
    );
}

#[test]
fn the_default_body_limit_still_guards_the_body_the_validator_reads() {
    let mut app = router().into_axum();
    let oversized = format!(
        r#"{{"slug":"ada","retries":2,"tags":["{}"]}}"#,
        "x".repeat(3_000_000)
    );
    let (status, body) = send(&mut app, json_request("/users", &oversized));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body.contains("a readable request body"),
        "{body} must report the body it refused to buffer"
    );
}

#[test]
fn a_request_that_satisfies_the_contract_reaches_the_handler() {
    let mut app = router().into_axum();
    for request in [
        json_request("/users", r#"{"slug":"ada","retries":2,"tags":["x","y"]}"#),
        get_request("/search?slug=ada&limit=5"),
        get_request("/search?slug=ada"),
        get_request("/users/ada-lovelace"),
        request(
            "/form",
            "application/x-www-form-urlencoded",
            "slug=ada-lovelace",
        ),
    ] {
        accepted(&mut app, request);
    }
    accepted(&mut app, header_request("/retried", "x-retries", "3"));
}

#[test]
fn every_validated_operation_declares_the_rejection_it_can_serve() {
    let document = router().into_document().unwrap();
    for literal in [
        "users",
        "search",
        "form",
        "retried",
        "flagged",
        "open-users",
        "typed-open",
    ] {
        let rejection = declared_rejection(&document, literal)
            .unwrap_or_else(|| panic!("/{literal} must declare the request rejection"));
        assert_eq!(rejection.status, ErrorStatus::Exact(422));
        assert_eq!(rejection.content, ErrorContent::Json);
        let Some(pons_ir::TypeRef::Named(report)) = &rejection.body else {
            panic!("/{literal} must declare the report body");
        };
        let published = body_type(&document, &report.id);
        assert_eq!(published.direction, Direction::Output);
        let TypeKind::Struct { fields, .. } = &published.definition else {
            panic!("/{literal} must declare a report object");
        };
        assert_eq!(fields[0].wire, "violations");
        let TypeRef::Inline(InlineType::Array { item, .. }) = &fields[0].ty else {
            panic!("/{literal} must declare one violation per entry");
        };
        let TypeRef::Named(violation) = &**item else {
            panic!("/{literal} must publish the shape of one violation");
        };
        let TypeKind::Struct { fields, .. } = &body_type(&document, &violation.id).definition
        else {
            panic!("/{literal} must publish a violation object");
        };
        assert_eq!(fields[0].wire, "path");
        assert_eq!(fields[1].wire, "reason");
    }
}

#[test]
fn an_operation_that_validates_nothing_declares_no_rejection() {
    let document = router().into_document().unwrap();
    for literal in ["note", "upload"] {
        assert!(
            declared_rejection(&document, literal).is_none(),
            "/{literal} validates nothing, so the document must not claim that it does"
        );
    }
    assert_eq!(
        operation(&document, "note").body.as_ref().unwrap().content,
        ContentKind::Text
    );
    assert_eq!(
        operation(&document, "upload")
            .body
            .as_ref()
            .unwrap()
            .content,
        ContentKind::Multipart
    );
}

#[test]
fn a_text_body_is_published_as_text_and_is_never_rejected() {
    let document = router().into_document().unwrap();
    let note = operation(&document, "note");
    assert_eq!(note.body.as_ref().unwrap().content, ContentKind::Text);
    assert!(note.errors.is_empty());

    let mut app = router().into_axum();
    for body in ["", "anything at all"] {
        accepted(&mut app, request("/note", "text/plain", body));
    }
}

#[test]
fn a_multipart_body_is_opaque_so_it_is_never_claimed_to_be_checked() {
    let document = router().into_document().unwrap();
    let upload = operation(&document, "upload");
    assert!(upload.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("multipart fields are opaque")
    }));
    assert!(upload.errors.is_empty());
}

#[test]
fn a_preserve_opaque_body_checks_its_fields_and_admits_it_accepts_unknown_properties() {
    let document = router().into_document().unwrap();
    assert_eq!(
        openness(&document, "validation::OpenUser#input"),
        ObjectOpenness::PreserveOpaque
    );

    let mut app = router().into_axum();
    accepted(
        &mut app,
        json_request("/open-users", r#"{"slug":"ada","nickname":"ada"}"#),
    );
    rejected(
        &mut app,
        json_request("/open-users", r#"{"slug":"Ada"}"#),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_preserve_typed_body_checks_the_extra_properties_it_declares() {
    let document = router().into_document().unwrap();
    let ObjectOpenness::PreserveTyped { value } =
        openness(&document, "validation::OpenRequest#input")
    else {
        panic!("the extra properties must be published as typed");
    };
    let TypeRef::Inline(InlineType::Map { value: extra, .. }) = value else {
        panic!("the extra properties must be published as a map of values");
    };
    assert!(matches!(
        *extra,
        TypeRef::Inline(InlineType::Int {
            minimum: Some(_),
            maximum: Some(_),
            ..
        })
    ));

    let mut app = router().into_axum();
    accepted(
        &mut app,
        json_request("/typed-open", r#"{"slug":"ada","extra":2}"#),
    );
    rejected(
        &mut app,
        json_request("/typed-open", r#"{"slug":"ada","extra":9}"#),
        &["/extra: integer is above its maximum of 5"],
    );
}

#[test]
fn a_stream_response_is_never_described_as_validated() {
    let document = router().into_document().unwrap();
    let events = document
        .operations
        .iter()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    assert_eq!(events.errors.len(), 1);
    assert_eq!(events.errors[0].name, ["request", "rejected"]);
    assert_eq!(events.responses.len(), 1);
    assert_eq!(events.responses[0].content, ContentKind::Stream);
    assert!(events.responses[0].body.is_none());
}

#[test]
fn a_type_the_document_does_not_publish_is_refused_instead_of_served_unvalidated() {
    let mut app = unpublished_router().into_axum();
    rejected(
        &mut app,
        json_request("/unpublished", "{}"),
        &[": unknown type unpublished::Type#input"],
    );

    let errors = unpublished_router().into_document().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("unpublished::Type#input")),
        "{errors:?} must name the type the handler asks for and never publishes"
    );
}
