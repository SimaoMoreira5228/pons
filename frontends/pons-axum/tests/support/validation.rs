use super::info;
use pons_axum::extract::Multipart;
use pons_axum::routing::{get, post};
use pons_axum::{Header, HeaderSpec, Json, Router};
use pons_core::{Schema, TypeCx};
use pons_ir::{
    ContentKind, Direction, Field, InlineType, IntegerWire, ObjectOpenness, Presence, TypeKind,
    TypeRef,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(transparent)]
pub struct Slug(pub String);

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
pub struct Retries(pub u8);

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

pub struct RetriesHeader;

impl HeaderSpec for RetriesHeader {
    type Value = Retries;
    const NAME: &'static str = "X-Retries";
}

pub struct DebugFlag;

impl HeaderSpec for DebugFlag {
    type Value = bool;
    const NAME: &'static str = "X-Debug";
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    pub slug: Slug,
    pub retries: Retries,
    pub tags: BTreeSet<String>,
}

#[derive(Deserialize, Schema)]
pub struct OpenUser {
    pub slug: Slug,
}

#[derive(Deserialize)]
pub struct OpenRequest {
    pub slug: Slug,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
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

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub slug: Slug,
    #[serde(default)]
    pub limit: Option<Retries>,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct UserPath {
    pub slug: Slug,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct SlugForm {
    pub slug: Slug,
}

#[derive(Serialize, Schema)]
pub struct User {
    pub id: u64,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct Both {
    pub slug: String,
    pub limit: Option<u16>,
}

pub async fn both_query(pons_axum::Query(query): pons_axum::Query<Both>) -> axum::Json<User> {
    let _ = (query.slug, query.limit);
    axum::Json(User { id: 1 })
}

pub async fn both_body(Json(body): Json<Both>) -> axum::Json<User> {
    let _ = (body.slug, body.limit);
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub cursor: Retries,
}

#[derive(Deserialize, Schema)]
#[serde(transparent)]
pub struct PageQuery(pub Page);

pub async fn newtype_query(
    pons_axum::Query(query): pons_axum::Query<PageQuery>,
) -> axum::Json<User> {
    let _ = query.0.cursor.0;
    axum::Json(User { id: 1 })
}

pub async fn newtype_form(pons_axum::Form(form): pons_axum::Form<PageQuery>) -> axum::Json<User> {
    let _ = form.0.cursor.0;
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct Feed {
    pub name: String,
    #[serde(flatten)]
    pub page: PageQuery,
}

pub async fn flattened_query(pons_axum::Query(query): pons_axum::Query<Feed>) -> axum::Json<User> {
    let _ = (query.name, query.page.0.cursor.0);
    axum::Json(User { id: 1 })
}

#[derive(Deserialize)]
pub struct Token(pub String);

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

pub struct TokenHeader;

impl HeaderSpec for TokenHeader {
    type Value = Token;
    const NAME: &'static str = "X-Token";
}

pub async fn tokenized(Header(token): Header<TokenHeader>) -> axum::Json<User> {
    let _ = token.0;
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct Tags {
    pub tags: Vec<Retries>,
}

pub async fn tagged_form(pons_axum::Form(form): pons_axum::Form<Tags>) -> axum::Json<User> {
    let _ = form.tags;
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct Labels {
    pub labels: Vec<String>,
}

pub async fn labeled_form(pons_axum::Form(form): pons_axum::Form<Labels>) -> axum::Json<User> {
    let _ = form.labels;
    axum::Json(User { id: 1 })
}

#[derive(Serialize, Schema)]
pub struct UserEvent {
    pub id: u64,
}

impl pons_axum::SseEvent for UserEvent {
    const NAME: &'static str = "user";
}

pub async fn create_user(Json(input): Json<CreateUser>) -> axum::Json<User> {
    let _ = (input.slug, input.retries, input.tags);
    axum::Json(User { id: 1 })
}

pub async fn create_open_user(Json(input): Json<OpenUser>) -> axum::Json<User> {
    let _ = input.slug;
    axum::Json(User { id: 1 })
}

pub async fn create_typed_open_request(Json(input): Json<OpenRequest>) -> axum::Json<User> {
    let _ = (input.slug, input.extra);
    axum::Json(User { id: 1 })
}

pub async fn search(pons_axum::Query(query): pons_axum::Query<Search>) -> axum::Json<User> {
    let _ = (query.slug.0, query.limit.map(|limit| limit.0));
    axum::Json(User { id: 1 })
}

pub async fn get_user(pons_axum::Path(path): pons_axum::Path<UserPath>) -> axum::Json<User> {
    let _ = path.slug.0;
    axum::Json(User { id: 1 })
}

pub async fn create_form_user(
    pons_axum::Form(form): pons_axum::Form<SlugForm>,
) -> axum::Json<User> {
    let _ = form.slug.0;
    axum::Json(User { id: 1 })
}

pub async fn retried(Header(retries): Header<RetriesHeader>) -> axum::Json<User> {
    let _ = retries.0;
    axum::Json(User { id: 1 })
}

pub async fn flagged(Header(flag): Header<DebugFlag>) -> axum::Json<User> {
    let _ = flag;
    axum::Json(User { id: 1 })
}

pub async fn note(_note: String) -> axum::Json<User> {
    axum::Json(User { id: 1 })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub text: String,
}

pub async fn upload(_upload: Multipart) -> axum::Json<User> {
    axum::Json(User { id: 1 })
}

pub async fn user_events(
    pons_axum::Query(query): pons_axum::Query<Search>,
) -> pons_axum::Sse<UserEvent, impl futures_util::Stream<Item = UserEvent> + Send> {
    let _ = (query.slug.0, query.limit.map(|limit| limit.0));
    pons_axum::Sse::new(futures_util::stream::iter([UserEvent { id: 1 }]))
}

pub struct VndNote<T>(pub T);

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

pub fn router() -> Router<()> {
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

pub fn carriers() -> Router<()> {
    Router::new(info())
        .route("/both-query", get(both_query))
        .route("/both-body", post(both_body))
        .route("/newtype-query", get(newtype_query))
        .route("/newtype-form", post(newtype_form))
        .route("/flattened", get(flattened_query))
        .route("/tags-form", post(tagged_form))
        .route("/labels-form", post(labeled_form))
}

pub fn headers() -> Router<()> {
    Router::new(info())
        .route("/retried", get(retried))
        .route("/token", get(tokenized))
}
