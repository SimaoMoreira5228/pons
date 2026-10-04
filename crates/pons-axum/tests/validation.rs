use axum::body::Body;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use pons_axum::extract::Multipart;
use pons_axum::http::Request;
use pons_axum::routing::{get, get_with, head, head_with, post, post_with};
use pons_axum::{
    Header, HeaderSpec, Info, Json, NoContent, OperationMeta, Router, WithErrorHeader,
};
use pons_core::{ErrorSpec, Schema, TypeCx};
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

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Both {
    slug: String,
    limit: Option<u16>,
}

async fn both_query(pons_axum::Query(query): pons_axum::Query<Both>) -> axum::Json<User> {
    let _ = (query.slug, query.limit);
    axum::Json(User { id: 1 })
}

async fn both_body(Json(body): Json<Both>) -> axum::Json<User> {
    let _ = (body.slug, body.limit);
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Page {
    cursor: Retries,
}

#[derive(Deserialize, Schema)]
#[serde(transparent)]
struct PageQuery(Page);

async fn newtype_query(pons_axum::Query(query): pons_axum::Query<PageQuery>) -> axum::Json<User> {
    let _ = query.0.cursor.0;
    axum::Json(User { id: 1 })
}

async fn newtype_form(pons_axum::Form(form): pons_axum::Form<PageQuery>) -> axum::Json<User> {
    let _ = form.0.cursor.0;
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Feed {
    name: String,
    #[serde(flatten)]
    page: PageQuery,
}

async fn flattened_query(pons_axum::Query(query): pons_axum::Query<Feed>) -> axum::Json<User> {
    let _ = (query.name, query.page.0.cursor.0);
    axum::Json(User { id: 1 })
}

#[derive(Deserialize)]
struct Token(String);

impl Schema for Token {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }
}

impl std::str::FromStr for Token {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.chars().all(|character| character.is_ascii_lowercase()) {
            true => Ok(Token(text.to_owned())),
            false => Err(()),
        }
    }
}

struct TokenHeader;

impl HeaderSpec for TokenHeader {
    type Value = Token;
    const NAME: &'static str = "X-Token";
}

async fn tokenized(Header(token): Header<TokenHeader>) -> axum::Json<User> {
    let _ = token.0;
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Tags {
    tags: Vec<Retries>,
}

async fn tagged_form(pons_axum::Form(form): pons_axum::Form<Tags>) -> axum::Json<User> {
    let _ = form.tags;
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Labels {
    labels: Vec<String>,
}

async fn labeled_form(pons_axum::Form(form): pons_axum::Form<Labels>) -> axum::Json<User> {
    let _ = form.labels;
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Ordered {
    zeta: Retries,
    alpha: Retries,
}

async fn ordered(Json(body): Json<Ordered>) -> axum::Json<User> {
    let _ = (body.zeta.0, body.alpha.0);
    axum::Json(User { id: 1 })
}

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
enum DeclaredRejection {
    #[pons(status = 422)]
    Rejected,
}

impl IntoResponse for DeclaredRejection {
    fn into_response(self) -> axum::response::Response {
        StatusCode::UNPROCESSABLE_ENTITY.into_response()
    }
}

async fn declares_its_own_rejection(
    Json(_body): Json<OpenUser>,
) -> Result<axum::Json<User>, DeclaredRejection> {
    Err(DeclaredRejection::Rejected)
}

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

async fn head_search(pons_axum::Query(_query): pons_axum::Query<Search>) -> NoContent {
    NoContent
}

async fn head_count(Header(_header): Header<RetriesHeader>) -> NoContent {
    NoContent
}

async fn ping() -> NoContent {
    NoContent
}

async fn unchecked_create_user(Json(_body): Json<CreateUser>) -> axum::Json<User> {
    axum::Json(User { id: 1 })
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

async fn form_on_get(pons_axum::Form(form): pons_axum::Form<SlugForm>) -> axum::Json<User> {
    axum::Json(User {
        id: form.slug.0.len() as u64,
    })
}

async fn raw_form_on_get(pons_axum::extract::RawForm(form): pons_axum::extract::RawForm) -> String {
    String::from_utf8_lossy(&form).into_owned()
}

async fn form_on_head(pons_axum::Form(form): pons_axum::Form<SlugForm>) -> NoContent {
    let _ = form.slug.0;
    NoContent
}

async fn scalar_form_on_get(pons_axum::Form(form): pons_axum::Form<u64>) -> axum::Json<User> {
    axum::Json(User { id: form })
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

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Note {
    text: String,
}

async fn note_body(Json(note): Json<Note>) -> axum::Json<User> {
    let _ = note.text;
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

fn carriers() -> Router<()> {
    Router::new(info())
        .route("/both-query", get(both_query))
        .route("/both-body", post(both_body))
        .route("/newtype-query", get(newtype_query))
        .route("/newtype-form", post(newtype_form))
        .route("/flattened", get(flattened_query))
        .route("/tags-form", post(tagged_form))
        .route("/labels-form", post(labeled_form))
}

fn headers() -> Router<()> {
    Router::new(info())
        .route("/retried", get(retried))
        .route("/token", get(tokenized))
}

fn ordered_router() -> Router<()> {
    Router::new(info()).route("/ordered", post(ordered))
}

fn declared_router() -> Router<()> {
    Router::new(info()).route("/declared", post(declares_its_own_rejection))
}

fn inherited_router() -> Router<()> {
    Router::new(info())
        .common_error::<CommonRejection>()
        .route("/inherited", post(create_open_user))
}

fn head_router() -> Router<()> {
    Router::new(info())
        .route("/head-search", head(head_search))
        .route("/head-count", head(head_count))
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

fn built(router: Router<()>) -> axum::Router<()> {
    router
        .into_axum()
        .expect("the fixture router installs a document that validates")
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
    let mut app = built(router());
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
    let mut app = built(router());
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","retries":9,"tags":["x"]}"#),
        &["/retries: integer is above its maximum of 5"],
    );
}

#[test]
fn a_body_missing_a_required_field_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","tags":["x"]}"#),
        &["/retries: missing required field retries"],
    );
}

#[test]
fn a_body_that_repeats_an_item_of_a_unique_array_is_rejected() {
    let mut app = built(router());
    rejected(
        &mut app,
        json_request("/users", r#"{"slug":"ada","retries":2,"tags":["x","x"]}"#),
        &["/tags: array item 1 repeats item 0"],
    );
}

#[test]
fn an_unknown_property_is_rejected_against_deny_unknown_fields() {
    let mut app = built(router());
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
    let mut app = built(router());
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
    let mut app = built(router());
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
    let mut app = built(router());
    rejected(
        &mut app,
        get_request("/users/Ada%20Lovelace"),
        &["/slug: string does not match the pattern ^[a-z][a-z0-9-]*$"],
    );
}

#[test]
fn a_form_field_outside_its_published_pattern_is_rejected() {
    let mut app = built(router());
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
    let mut app = built(router());
    rejected(
        &mut app,
        header_request("/retried", "x-retries", "9"),
        &[": integer is above its maximum of 5"],
    );
}

#[test]
fn a_header_that_is_not_the_text_token_its_type_reads_is_rejected() {
    let mut app = built(router());
    accepted(&mut app, header_request("/flagged", "x-debug", "true"));
    rejected(
        &mut app,
        header_request("/flagged", "x-debug", "1"),
        &[": expected a boolean, found a string"],
    );
}

#[test]
fn the_default_body_limit_still_guards_the_body_the_validator_reads() {
    let mut app = built(router());
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
    let mut app = built(router());
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

    let mut app = built(router());
    for body in ["", "anything at all"] {
        accepted(&mut app, request("/note", "text/plain", body));
    }
}

#[test]
fn a_text_body_the_framework_reads_is_published_as_optional_and_says_what_it_rejects() {
    let document = Router::<()>::new(info())
        .route("/note", post(note))
        .into_document()
        .unwrap();
    let text = operation(&document, "note");
    assert!(
        !text.body.as_ref().unwrap().required,
        "a request with no body at all reaches the handler as an empty string"
    );
    assert!(
        text.errors.is_empty(),
        "the framework reads this body, so nothing pons validates is rejected: {:?}",
        text.errors
    );
    let message = text
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-W101")
        .map(|diagnostic| diagnostic.message.as_str())
        .expect("a body the framework reads earns the rejections it can answer");
    assert!(
        message.contains("400") && message.contains("413"),
        "{message} must name the statuses the framework answers before the handler runs"
    );
    assert!(
        message.contains("declares none of"),
        "{message} must agree with what the operation declares"
    );

    let mut app = built(Router::<()>::new(info()).route("/note", post(note)));
    let empty: Request<Body> = Request::builder()
        .method("POST")
        .uri("/note")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&mut app, empty);
    assert_eq!(status, StatusCode::OK, "{body}");

    let undecodable: Request<Body> = Request::builder()
        .method("POST")
        .uri("/note")
        .body(Body::from(&[0xff, 0xfe][..]))
        .unwrap();
    let (status, body) = send(&mut app, undecodable);
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let oversized: Request<Body> = Request::builder()
        .method("POST")
        .uri("/note")
        .body(Body::from(vec![b'a'; 3_000_000]))
        .unwrap();
    let (status, body) = send(&mut app, oversized);
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
}

#[test]
fn a_form_on_a_get_is_published_as_the_query_the_runtime_reads() {
    let document = Router::<()>::new(info())
        .route("/form", get(form_on_get))
        .into_document()
        .unwrap();
    let operation = &document.operations[0];
    assert!(
        operation.body.is_none(),
        "a form on a GET is read from the query, so the document publishes no request body"
    );
    let TypeRef::Named(form) = operation.query.as_ref().unwrap() else {
        panic!("a typed form is published as a named type")
    };
    assert!(
        form.id.ends_with("#form"),
        "the runtime resolves the carrier the document publishes: {}",
        form.id
    );

    let mut app = built(Router::<()>::new(info()).route("/form", get(form_on_get)));
    let (status, body) = send(&mut app, get_request("/form?slug=ada-lovelace"));
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("\"id\":12"),
        "{body} must be read from the query the document publishes"
    );

    let errors = refused(Router::<()>::new(info()).route("/raw-form", get(raw_form_on_get)));
    assert!(
        errors.contains("PONS-E102") && errors.contains("query"),
        "{errors} must refuse a query this contract cannot publish opaquely"
    );
}

#[test]
fn a_form_on_a_head_is_published_as_the_query_the_runtime_reads() {
    fn router() -> Router<()> {
        Router::new(info()).route(
            "/head-form",
            head_with(form_on_head, OperationMeta::new("head.form")),
        )
    }
    let document = router().into_document().unwrap();
    let operation = &document.operations[0];
    assert!(
        operation.body.is_none(),
        "a form on a HEAD is read from the query, so the document publishes no request body: {:?}",
        operation.body
    );
    let TypeRef::Named(form) = operation.query.as_ref().unwrap() else {
        panic!("a typed form is published as a named type")
    };
    assert!(
        form.id.ends_with("#form"),
        "the runtime resolves the carrier the document publishes: {}",
        form.id
    );

    let mut app = built(router());
    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("HEAD")
            .uri("/head-form?slug=ada-lovelace")
            .body(Body::empty())
            .unwrap(),
    );
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = send(
        &mut app,
        Request::builder()
            .method("HEAD")
            .uri("/head-form")
            .body(Body::empty())
            .unwrap(),
    );
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a HEAD the query does not satisfy is rejected as the report the operation declares: {body}"
    );

    let errors = refused(Router::new(info()).route(
        "/raw-head-form",
        head_with(raw_form_on_get, OperationMeta::new("head.raw_form")),
    ));
    assert!(
        errors.contains("PONS-E102") && errors.contains("struct"),
        "{errors} must refuse the opaque form type, whose fields a query cannot spell"
    );
}

#[test]
fn a_form_that_becomes_a_query_must_be_a_struct() {
    let errors = refused(Router::new(info()).route("/scalar", get(scalar_form_on_get)));
    assert!(
        errors.contains("PONS-E102") && errors.contains("struct"),
        "{errors} must name the struct the query slot a GET form becomes has to be"
    );
    assert!(
        !errors.contains("use a typed form"),
        "{errors} must not tell a user who already used a typed form to use one"
    );
}

#[test]
fn a_multipart_body_on_a_get_is_refused_because_axum_answers_400_first() {
    let errors = refused(Router::<()>::new(info()).route("/upload", get(upload)));
    assert!(
        errors.contains("PONS-E102") && errors.contains("boundary"),
        "{errors} must name the boundary a GET request never carries"
    );

    let mut app = axum::Router::new().route("/upload", axum::routing::get(upload));
    let (status, body) = send(&mut app, get_request("/upload"));
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains("boundary"),
        "{body} is the rejection the refusal names"
    );
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

    let mut app = built(router());
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

    let mut app = built(router());
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
    let errors = unpublished_router().into_document().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("unpublished::Type#input")),
        "{errors:?} must name the type the handler asks for and never publishes"
    );
    assert!(
        refused_build(unpublished_router()).contains("unpublished::Type#input"),
        "into_axum returns the errors instead of panicking at startup on a document that does not validate"
    );
}

#[test]
fn a_json_body_keeps_the_nullability_a_query_carrier_lowered() {
    let mut app = built(carriers());

    accepted(
        &mut app,
        json_request("/both-body", r#"{"slug":"ada","limit":null}"#),
    );
    accepted(
        &mut app,
        json_request("/both-body", r#"{"slug":"ada","limit":2}"#),
    );
    rejected(
        &mut app,
        json_request("/both-body", r#"{"slug":"ada","limit":"two"}"#),
        &["/limit: expected a number, found a string"],
    );
}

#[test]
fn a_carrier_newtype_reads_the_struct_it_wraps() {
    let mut app = built(carriers());
    accepted(&mut app, get_request("/newtype-query?cursor=2"));
    accepted(
        &mut app,
        request(
            "/newtype-form",
            "application/x-www-form-urlencoded",
            "cursor=2",
        ),
    );
    rejected(
        &mut app,
        get_request("/newtype-query?cursor=9"),
        &["/cursor: integer is above its maximum of 5"],
    );
    rejected(
        &mut app,
        request(
            "/newtype-form",
            "application/x-www-form-urlencoded",
            "cursor=9",
        ),
        &["/cursor: integer is above its maximum of 5"],
    );
}

#[test]
fn a_newtype_wrapped_flattened_member_reads_its_own_fields() {
    let mut app = built(carriers());
    accepted(&mut app, get_request("/flattened?name=ada&cursor=2"));
    rejected(
        &mut app,
        get_request("/flattened?name=ada&cursor=9"),
        &["/cursor: integer is above its maximum of 5"],
    );
}

#[test]
fn an_array_field_reads_either_spelling_a_carrier_uses() {
    fn form(body: &str) -> Request<Body> {
        request("/tags-form", "application/x-www-form-urlencoded", body)
    }
    let mut app = built(carriers());

    for body in ["tags=1&tags=2", "tags=%5B1%2C2%5D", "tags=1"] {
        accepted(&mut app, form(body));
    }
    for body in ["tags=1&tags=9", "tags=%5B1%2C9%5D", "tags=nine"] {
        let expected: &[&str] = match body {
            "tags=nine" => &["/tags/0: expected a number, found a string"],
            _ => &["/tags/1: integer is above its maximum of 5"],
        };
        rejected(&mut app, form(body), expected);
    }
}

#[test]
fn a_repeated_field_that_is_not_an_array_is_not_reported_as_a_duplicate_array_item() {
    let mut app = built(carriers());
    rejected(
        &mut app,
        get_request("/newtype-query?cursor=1&cursor=2"),
        &["/cursor: expected one value for a field that is not an array, found 2 values"],
    );
    rejected(
        &mut app,
        get_request("/newtype-query?cursor=1&cursor=1&cursor=2"),
        &["/cursor: expected one value for a field that is not an array, found 3 values"],
    );
}

#[test]
fn an_operation_that_declares_the_rejection_status_reuses_that_slot() {
    let document = declared_router().into_document().unwrap();
    let declared = operation(&document, "declared");
    let rejected: Vec<_> = declared
        .errors
        .iter()
        .filter(|error| error.status == ErrorStatus::Exact(422))
        .collect();
    assert_eq!(
        rejected.len(),
        1,
        "{:?} declares the status once",
        declared.errors
    );
    assert_eq!(
        rejected[0].name,
        ["rejected"],
        "the slot the operation declares is kept"
    );
    assert!(
        declared.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PONS-W101" && diagnostic.message.contains("\"rejected\"")
        }),
        "{:?} must name the error a request is now rejected as",
        declared.diagnostics
    );
    assert!(document.diagnostics.is_empty());
}

struct RequestId;

impl HeaderSpec for RequestId {
    type Value = String;
    const NAME: &'static str = "X-Request-ID";
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

fn refused(router: Router<()>) -> String {
    let errors = router.into_document().unwrap_err();
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn refused_build(router: Router<()>) -> String {
    let errors = router
        .into_axum()
        .expect_err("a router that installs a document which does not validate must not be built");
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
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
fn a_head_operation_declares_the_rejection_without_a_body() {
    let document = head_router().into_document().unwrap();
    for literal in ["head-search", "head-count"] {
        let rejection = declared_rejection(&document, literal)
            .unwrap_or_else(|| panic!("/{literal} must declare the request rejection"));
        assert_eq!(rejection.status, ErrorStatus::Exact(422));
        assert_eq!(rejection.content, ErrorContent::Empty);
        assert!(
            rejection.body.is_none(),
            "/{literal} cannot carry a rejection body"
        );
        assert!(rejection.media_type.is_none());
    }
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
fn a_body_that_announces_another_content_type_is_rejected() {
    let mut app = built(router());
    let (status, body) = send(
        &mut app,
        request(
            "/users",
            "text/plain",
            r#"{"slug":"ada","retries":2,"tags":["x"]}"#,
        ),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/json, found text/plain"
    );
}

#[test]
fn a_body_that_announces_the_published_content_type_is_accepted() {
    let mut app = built(router());
    for content_type in ["application/json", "application/json; charset=utf-8"] {
        accepted(
            &mut app,
            request(
                "/users",
                content_type,
                r#"{"slug":"ada","retries":2,"tags":["x"]}"#,
            ),
        );
    }
}

#[test]
fn every_header_rejection_answers_the_status_the_operation_declares() {
    let document = headers().into_document().unwrap();
    assert_eq!(
        declared_rejection(&document, "retried").map(|rejection| rejection.status),
        Some(ErrorStatus::Exact(422)),
        "a typed header declares the rejection it can serve"
    );

    let mut app = built(headers());

    rejected(
        &mut app,
        get_request("/retried"),
        &[": expected a request carrying the required header, found no header"],
    );

    let mut repeated = get_request("/retried");
    repeated
        .headers_mut()
        .append("x-retries", "1".parse().unwrap());
    repeated
        .headers_mut()
        .append("x-retries", "2".parse().unwrap());
    rejected(
        &mut app,
        repeated,
        &[": expected a header spelled once, found a repeated header"],
    );

    rejected(
        &mut app,
        header_request("/retried", "x-retries", "9"),
        &[": integer is above its maximum of 5"],
    );

    rejected(
        &mut app,
        header_request("/token", "x-token", "Ada"),
        &[": expected a header value the type reads, found a value it cannot read"],
    );
    accepted(&mut app, header_request("/token", "x-token", "ada"));
}

#[test]
fn a_validated_extractor_driven_without_the_document_is_refused() {
    let mut app =
        axum::Router::new().route("/undocumented", axum::routing::post(unchecked_create_user));
    let (status, body) = send(
        &mut app,
        json_request(
            "/undocumented",
            r#"{"slug":"ada","retries":2,"tags":["x"]}"#,
        ),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected a request carrying the document Router::into_axum installs, found a request carrying none"
    );
}

#[test]
fn the_runtime_checks_a_request_in_the_order_the_document_publishes() {
    let document = ordered_router().into_document().unwrap();
    let TypeRef::Named(body) = &operation(&document, "ordered").body.as_ref().unwrap().ty else {
        panic!("a JSON body is published as a named type")
    };
    let TypeKind::Struct { fields, .. } = &document.types[&body.id].definition else {
        panic!("a JSON body is published as a struct")
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.wire.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"],
        "the published document sorts the fields it publishes"
    );

    let mut app = built(ordered_router());
    rejected(
        &mut app,
        json_request("/ordered", r#"{"zeta":9,"alpha":9}"#),
        &[
            "/alpha: integer is above its maximum of 5",
            "/zeta: integer is above its maximum of 5",
        ],
    );
}

#[test]
fn the_extract_module_reaches_the_extractors_that_check_the_contract() {
    async fn through_the_module(
        pons_axum::extract::Json(body): pons_axum::extract::Json<Ordered>,
    ) -> axum::Json<User> {
        let _ = (body.zeta.0, body.alpha.0);
        axum::Json(User { id: 1 })
    }
    let mut app = built(Router::<()>::new(info()).route("/module", post(through_the_module)));
    accepted(&mut app, json_request("/module", r#"{"zeta":1,"alpha":2}"#));
    rejected(
        &mut app,
        json_request("/module", r#"{"zeta":9,"alpha":2}"#),
        &["/zeta: integer is above its maximum of 5"],
    );
}

#[test]
fn a_request_that_carries_no_content_type_is_read_rather_than_rejected() {
    let mut app = built(
        Router::<()>::new(info())
            .route(
                "/note",
                get_with(note_body, OperationMeta::new("note.read")),
            )
            .route(
                "/posted",
                post_with(note_body, OperationMeta::new("note.posted")),
            ),
    );

    let (status, body) = send(&mut app, get_request("/note"));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert!(
        report["violations"][0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("expected a JSON document"),
        "{body} must report the body it could not read, not a content type it never sent"
    );

    let mut announced = get_request("/note");
    announced
        .headers_mut()
        .insert("content-type", "text/plain".parse().unwrap());
    let (status, body) = send(&mut app, announced);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"], "expected application/json, found text/plain",
        "a content type the request announces is still the one the document demands"
    );

    let mut wrong_body = json_request("/posted", r#"{"text":"hi"}"#);
    wrong_body
        .headers_mut()
        .insert("content-type", "text/plain".parse().unwrap());
    let (status, body) = send(&mut app, wrong_body);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/json, found text/plain"
    );

    let unannounced = Request::builder()
        .method("POST")
        .uri("/posted")
        .body(Body::from(r#"{"text":"hi"}"#.to_owned()))
        .unwrap();
    let (status, body) = send(&mut app, unannounced);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"], "expected application/json, found ",
        "a payload the request carries is still the one the document demands"
    );
}

struct VndNote<T>(pub T);

impl<T, S> axum::extract::FromRequest<S> for VndNote<T>
where
    T: Schema + serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = pons_axum::ViolationReport;

    async fn from_request(
        request: pons_axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(note)| VndNote(note))
    }
}

impl<T: Schema> pons_axum::DescribeRequest for VndNote<T> {
    fn describe(cx: &mut pons_axum::OperationCx) {
        cx.request_body_with_media::<T>(
            ContentKind::Json,
            Some("application/vnd.note+json".into()),
        );
        cx.mark_validated();
    }
}

async fn vnd_note(VndNote(note): VndNote<Note>) -> axum::Json<User> {
    let _ = note.text;
    axum::Json(User { id: 1 })
}

async fn plain_item(Json(note): Json<Note>) -> axum::Json<User> {
    let _ = note.text;
    axum::Json(User { id: 1 })
}

#[test]
fn a_body_is_accepted_with_the_media_type_its_operation_declares() {
    let document = Router::<()>::new(info())
        .route("/notes", post(vnd_note))
        .into_document()
        .unwrap();
    let body = operation(&document, "notes").body.as_ref().unwrap();
    assert_eq!(body.content, ContentKind::Json);
    assert_eq!(
        body.media_type.as_deref(),
        Some("application/vnd.note+json")
    );

    let mut app = built(Router::<()>::new(info()).route("/notes", post(vnd_note)));
    accepted(
        &mut app,
        request("/notes", "application/vnd.note+json", r#"{"text":"hi"}"#),
    );
    let (status, body) = send(
        &mut app,
        request("/notes", "application/json", r#"{"text":"hi"}"#),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/vnd.note+json, found application/json"
    );
}

#[test]
fn the_media_type_of_the_route_the_router_picks_is_the_one_it_enforces() {
    fn router() -> Router<()> {
        Router::new(info())
            .route("/items/{id}", post(plain_item))
            .route("/items/special", post(vnd_note))
    }
    let document = router().into_document().unwrap();
    assert_eq!(
        operation(&document, "special")
            .body
            .as_ref()
            .unwrap()
            .media_type
            .as_deref(),
        Some("application/vnd.note+json")
    );

    let mut app = built(router());
    accepted(
        &mut app,
        request(
            "/items/special",
            "application/vnd.note+json",
            r#"{"text":"hi"}"#,
        ),
    );
    let (status, body) = send(
        &mut app,
        request("/items/special", "application/json", r#"{"text":"hi"}"#),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/vnd.note+json, found application/json",
        "the static route's media type is the one the runtime enforces"
    );
    accepted(
        &mut app,
        request("/items/other", "application/json", r#"{"text":"hi"}"#),
    );
}

#[test]
fn both_array_spellings_read_one_element_the_same_way() {
    fn form(body: &str) -> Request<Body> {
        request("/labels-form", "application/x-www-form-urlencoded", body)
    }
    let mut app = built(carriers());

    let numbers: &[&str] = &[
        "/labels/0: expected a string, found a number",
        "/labels/1: expected a string, found a number",
    ];
    rejected(&mut app, form("labels=%5B1%2C2%5D"), numbers);
    rejected(&mut app, form("labels=%5B1%2C2%5D&labels=3"), numbers);
    rejected(
        &mut app,
        form("labels=%5B1%2C2%5D&labels=%5B3%2C4%5D"),
        &[
            "/labels/0: expected a string, found a number",
            "/labels/1: expected a string, found a number",
            "/labels/2: expected a string, found a number",
            "/labels/3: expected a string, found a number",
        ],
    );
    for body in ["labels=a&labels=b", "labels=%5B%22a%22%2C%22b%22%5D"] {
        accepted(&mut app, form(body));
    }
}

#[test]
fn the_media_type_of_the_route_the_router_resolves_is_the_one_it_enforces() {
    fn router() -> Router<()> {
        Router::<()>::new(info())
            .route("/{lead}/x/y", post(plain_item))
            .route("/x/{first}/{second}", post(vnd_note))
    }
    let document = router().into_document().unwrap();
    let ids: Vec<Vec<Vec<String>>> = document
        .operations
        .iter()
        .map(|operation| operation.id.clone())
        .collect();
    assert_eq!(
        ids,
        [
            vec![vec!["plain".to_owned(), "item".to_owned()]],
            vec![vec!["vnd".to_owned(), "note".to_owned()]]
        ],
        "the operation ids sort the other way than the routes resolve"
    );

    let mut app = built(router());
    accepted(
        &mut app,
        request("/x/x/y", "application/vnd.note+json", r#"{"text":"hi"}"#),
    );
    let (status, body) = send(
        &mut app,
        request("/x/x/y", "application/json", r#"{"text":"hi"}"#),
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"],
        "expected application/vnd.note+json, found application/json",
        "the route axum resolves is the one whose media type is enforced"
    );
    accepted(
        &mut app,
        request("/z/x/y", "application/json", r#"{"text":"hi"}"#),
    );
}

#[test]
fn a_wildcard_path_segment_is_refused_instead_of_published_as_one_parameter() {
    fn router() -> Router<()> {
        Router::<()>::new(info()).route("/files/{*rest}", post(vnd_note))
    }
    let document = router().document();
    let files = operation(&document, "files");
    let diagnostic = files
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-E102")
        .expect("a wildcard segment cannot be described, so it must be diagnosed");
    assert_eq!(
        diagnostic.severity,
        pons_ir::DiagnosticSeverity::Error,
        "{diagnostic:?} must be an error, not a warning about a route nobody reads"
    );
    assert!(
        diagnostic.message.contains("wildcard"),
        "{} must name the wildcard it cannot publish",
        diagnostic.message
    );

    let errors = refused(router());
    assert!(
        errors.contains("PONS-E102") && errors.contains("wildcard"),
        "{errors} must refuse a route the IR cannot express"
    );
    assert!(
        refused_build(router()).contains("PONS-E102"),
        "a router that would install a document lying about its own routes must refuse to be built"
    );
}

#[test]
fn a_path_parameter_is_not_the_operation_beyond_its_own_segment() {
    fn router() -> Router<()> {
        Router::<()>::new(info())
            .route("/assets/{id}", post(vnd_note))
            .route("/assets/{id}/raw", post(plain_item))
    }
    let document = router().into_document().unwrap();
    let published: BTreeMap<String, Option<String>> = document
        .operations
        .iter()
        .map(|operation| {
            let path = operation
                .path
                .iter()
                .map(|segment| match segment {
                    PathSegment::Literal { lit } => lit.clone(),
                    PathSegment::Param { param, .. } => format!("{{{param}}}"),
                })
                .collect::<Vec<_>>()
                .join("/");
            (
                path,
                operation
                    .body
                    .as_ref()
                    .and_then(|body| body.media_type.clone()),
            )
        })
        .collect();
    assert_eq!(
        published.get("assets/{id}"),
        Some(&Some("application/vnd.note+json".to_owned()))
    );
    assert_eq!(
        published.get("assets/{id}/raw"),
        Some(&None),
        "a body that names no media type answers the content kind's, application/json"
    );

    let mut app = built(router());
    accepted(
        &mut app,
        request("/assets/1", "application/vnd.note+json", r#"{"text":"hi"}"#),
    );
    accepted(
        &mut app,
        request("/assets/1/raw", "application/json", r#"{"text":"hi"}"#),
    );
    rejected(
        &mut app,
        request(
            "/assets/1/raw",
            "application/vnd.note+json",
            r#"{"text":"hi"}"#,
        ),
        &[": expected application/json, found application/vnd.note+json"],
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
                    error.media_type = Some("application/json".into());
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
        document.common_errors[0].media_type.as_deref(),
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
                    error.media_type = Some("application/problem+json".into());
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
        rejection.media_type, None,
        "the runtime answers the rejection as the content kind spells it, application/json"
    );
    assert_eq!(
        document.common_errors[0].media_type.as_deref(),
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
            .contains("validation::CommonRejection#output")
            && diagnostic
                .message
                .contains("pons_axum::error::ViolationReport#output"),
        "{} must name the two error type ids",
        diagnostic.message
    );
}

#[derive(Deserialize, Schema)]
#[serde(rename_all = "snake_case")]
enum Level {
    Info,
    Warn,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Levels {
    levels: Vec<Level>,
}

async fn levels(pons_axum::Form(form): pons_axum::Form<Levels>) -> axum::Json<User> {
    let _ = form.levels;
    axum::Json(User { id: 1 })
}

#[test]
fn a_string_representation_enum_is_read_as_the_array_item_it_is() {
    let router = || {
        Router::<()>::new(info())
            .route("/tags-form", post(tagged_form))
            .route("/levels-form", post(levels))
    };
    let mut app = built(router());
    let form = |path: &str, body: &str| request(path, "application/x-www-form-urlencoded", body);

    for body in [
        "levels=info",
        "levels=info&levels=warn",
        "levels=%5B%22info%22%5D",
    ] {
        accepted(&mut app, form("/levels-form", body));
    }
    let (status, body) = send(&mut app, form("/levels-form", "levels=info&levels=nope"));
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        report["violations"][0]["reason"], "unknown enum variant nope",
        "an item a string enum cannot name is reported against the item"
    );
}

#[test]
fn a_repeated_key_reads_an_empty_value_as_the_item_it_spells() {
    let form = |body: &str| request("/tags-form", "application/x-www-form-urlencoded", body);
    let mut app = built(carriers());

    for (body, path) in [
        ("tags=", "/tags/0"),
        ("tags=&tags=1", "/tags/0"),
        ("tags=1&tags=", "/tags/1"),
    ] {
        let (status, text) = send(&mut app, form(body));
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {text}");
        let report: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            report["violations"][0]["path"], path,
            "{body} spells one empty item, which the integer item type refuses"
        );
    }
    for body in ["tags=%5B%5D", "tags=1&tags=2"] {
        accepted(&mut app, form(body));
    }
}
