use std::collections::BTreeMap;

use pons_axum::{Info, Router, export_main, routing::get};
use pons_ir::{Document, ValidationError};

async fn health() {}

fn build() -> Router {
    Router::new(Info {
        title: "Example".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    })
    .route("/health", get(health))
}

fn lower(mut document: Document) -> Result<Document, Vec<ValidationError>> {
    document.ext.insert("exported".into(), true.into());
    Ok(document)
}

export_main!(build, lower);
