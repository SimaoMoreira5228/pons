mod auth;
mod describe;
mod document;
mod error;
mod handler;
mod header;
mod media;
mod metadata;
#[cfg(feature = "macros")]
mod mount;
#[cfg(feature = "multipart")]
mod multipart;
#[cfg(feature = "ndjson")]
mod ndjson;
mod operation;
mod operation_cx;
mod request;
#[cfg(feature = "typed-header")]
mod response_header;
mod router;
#[cfg(feature = "sse")]
mod sse;
mod validate;
#[cfg(feature = "websocket")]
mod websocket;

pub use auth::{Auth, AuthSpec, SessionAuth};
pub use describe::{DescribeRequest, DescribeResponse, OperationCx, WithStatus};
pub use error::{Violation, ViolationReport};
pub use handler::DescribeHandler;
pub use header::{Header, HeaderRejection, HeaderSpec, OptionalHeader};
#[cfg(feature = "macros")]
#[doc(hidden)]
pub use linkme;
pub use media::{MediaType, WithMedia};
pub use metadata::OperationMeta;
#[cfg(feature = "macros")]
pub use mount::PONS_TAKO_ROUTES;
#[cfg(feature = "multipart")]
pub use multipart::PonsMultipart;
#[cfg(feature = "ndjson")]
pub use ndjson::PonsNdjson;
pub use pons_ir::{Document, Info, ValidationError};
#[cfg(feature = "macros")]
pub use pons_tako_macros::{delete, get, head, options, patch, post, put, route};
pub use request::{Form, Json, Path, Query};
#[cfg(feature = "typed-header")]
pub use response_header::{
    WithErrorHeader, WithHeader, WithOptionalErrorHeader, WithOptionalHeader,
};
pub use router::Router;
#[cfg(feature = "sse")]
pub use sse::PonsSse;
#[cfg(feature = "websocket")]
pub use websocket::{PonsWebSocket, TypedSocket, TypedSocketError};

pub use tako::extractors;
pub use tako::route::Route;
pub use tako::{Method, StatusCode};
