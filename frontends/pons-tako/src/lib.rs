mod auth;
mod describe;
mod document;
mod error;
mod handler;
mod metadata;
#[cfg(feature = "macros")]
mod mount;
#[cfg(feature = "multipart")]
mod multipart;
mod operation;
mod operation_cx;
mod request;
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
#[cfg(feature = "macros")]
#[doc(hidden)]
pub use linkme;
pub use metadata::OperationMeta;
#[cfg(feature = "macros")]
pub use mount::PONS_TAKO_ROUTES;
#[cfg(feature = "multipart")]
pub use multipart::PonsMultipart;
pub use pons_ir::{Document, Info, ValidationError};
#[cfg(feature = "macros")]
pub use pons_tako_macros::{delete, get, head, options, patch, post, put, route};
pub use request::{Form, Json, Path, Query};
pub use router::Router;
#[cfg(feature = "sse")]
pub use sse::PonsSse;
#[cfg(feature = "websocket")]
pub use websocket::{PonsWebSocket, TypedSocket, TypedSocketError};

pub use tako::extractors;
pub use tako::route::Route;
pub use tako::{Method, StatusCode};
