mod dialect;
mod document;
mod error;
mod media;
mod metadata;
mod naming;
mod operations;
mod parameters;
mod refs;
mod request_body;
mod responses;
mod schemas;
mod security;

pub use error::ImportError;
use pons_ir::Document;

pub fn import(bytes: &[u8]) -> Result<Document, ImportError> {
    document::import(bytes)
}
