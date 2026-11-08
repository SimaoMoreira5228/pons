mod auth;
mod describe;
mod document;
mod handler;
mod metadata;
#[cfg(feature = "macros")]
mod mount;
mod operation;
mod operation_cx;
mod router;
#[cfg(feature = "sse")]
mod sse;

pub use auth::{Auth, AuthSpec, SessionAuth};
pub use describe::{DescribeRequest, DescribeResponse, OperationCx, WithStatus};
pub use handler::DescribeHandler;
#[cfg(feature = "macros")]
#[doc(hidden)]
pub use linkme;
pub use metadata::OperationMeta;
#[cfg(feature = "macros")]
pub use mount::PONS_TAKO_ROUTES;
pub use pons_ir::{Document, Info, ValidationError};
#[cfg(feature = "macros")]
pub use pons_tako_macros::{delete, get, head, options, patch, post, put, route};
pub use router::Router;
#[cfg(feature = "sse")]
pub use sse::PonsSse;

pub use tako::extractors;
pub use tako::route::Route;
pub use tako::{Method, StatusCode};
