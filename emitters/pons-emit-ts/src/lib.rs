mod analysis;
mod client;
mod errors;
mod extensions;
mod naming;
mod options;
mod pagination;
mod render;
mod runtime;
mod types;
mod validation;

use std::collections::BTreeMap;

use pons_ir::{
    ApiKeyLocation, AuthScheme, Coverage, Document, StreamKind, ValidationCoverage,
    coverage_for_document, operation_subject,
};

use crate::analysis::{
    camel_field_keys, scan_type_references, strict_coverage_error, strict_validation_error,
    subjects_reaching_a_decimal, validation_warnings,
};
use crate::naming::type_names;

pub use options::{
    DateMode, EmitMode, ErrorStrategy, FieldCase, Int64, Namespaces, Newtypes, Runtime, TsOptions,
    ValidationBackend, ValidationOptions,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitError(pub String);

impl std::fmt::Display for EmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EmitError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitOutput {
    pub files: Vec<GeneratedFile>,
    pub warnings: Vec<String>,
    pub coverage: Coverage,
    pub validation_coverage: ValidationCoverage,
}

pub fn emit(document: &Document) -> Result<Vec<GeneratedFile>, EmitError> {
    emit_with_options(document, TsOptions::default()).map(|output| output.files)
}

pub fn emit_with_mode(document: &Document, mode: EmitMode) -> Result<EmitOutput, EmitError> {
    emit_with_options(
        document,
        TsOptions {
            mode,
            ..TsOptions::default()
        },
    )
}

pub fn emit_with_strategy(
    document: &Document,
    mode: EmitMode,
    error_strategy: ErrorStrategy,
) -> Result<EmitOutput, EmitError> {
    emit_with_options(
        document,
        TsOptions {
            mode,
            errors: error_strategy,
            ..TsOptions::default()
        },
    )
}

pub fn emit_with_options(document: &Document, options: TsOptions) -> Result<EmitOutput, EmitError> {
    let mode = options.mode;
    let mut document = document.clone();
    document.canonicalize();
    document.validate().map_err(|errors| {
        EmitError(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    if document.operations.iter().any(|operation| {
        operation.responses.iter().any(|response| {
            matches!(
                response.content,
                pons_ir::ContentKind::Form | pons_ir::ContentKind::Multipart
            )
        })
    }) {
        return Err(EmitError(
            "the TypeScript emitter does not support form or multipart responses".into(),
        ));
    }
    if document.operations.iter().any(|operation| {
        operation.body.as_ref().is_some_and(|body| {
            body.content == pons_ir::ContentKind::Multipart && body.media.is_some()
        })
    }) {
        return Err(EmitError(
            "the TypeScript emitter does not support multipart media type overrides".into(),
        ));
    }
    let media_codecs = media_codec_paths(&document);
    if !media_codecs.is_empty() {
        return Err(EmitError(format!(
            "the TypeScript emitter cannot honor the media codec ids at {}",
            media_codecs.join(", ")
        )));
    }
    if document.operations.iter().any(|operation| {
        operation.stream.as_ref().is_some_and(|stream| {
            !matches!(
                &stream.kind,
                StreamKind::Sse { .. } | StreamKind::Ndjson { .. } | StreamKind::Websocket { .. }
            )
        }) || operation.stream.is_none()
            && operation
                .responses
                .iter()
                .any(|response| response.content == pons_ir::ContentKind::Stream)
    }) {
        return Err(EmitError(
            "the TypeScript emitter supports SSE, NDJSON, and WebSocket streams only".into(),
        ));
    }
    if document.operations.iter().any(|operation| {
        operation.body.is_some()
            && operation
                .stream
                .as_ref()
                .is_some_and(|stream| matches!(stream.kind, StreamKind::Websocket { .. }))
    }) {
        return Err(EmitError(
            "a WebSocket operation cannot declare a request body, because the upgrade request does not carry one".into(),
        ));
    }
    let api_key_headers: BTreeMap<String, &str> = document
        .auth_schemes
        .iter()
        .filter_map(|(id, scheme)| match scheme {
            AuthScheme::ApiKey {
                location: ApiKeyLocation::Header,
                name,
            } => Some((name.to_ascii_lowercase(), id.as_str())),
            _ => None,
        })
        .collect();
    for (index, operation) in document.operations.iter().enumerate() {
        for header in &operation.headers {
            let name = header.wire.to_ascii_lowercase();
            if let Some(scheme) = api_key_headers.get(&name) {
                return Err(EmitError(format!(
                    "the TypeScript emitter rejects the request header at operations[{index}].headers that the {scheme} credential overwrites: {}",
                    header.wire
                )));
            }
        }
    }
    let mut coverage = coverage_for_document(
        &document,
        mode == EmitMode::Lossy || options.int64 == Int64::Number,
    )
    .map_err(EmitError)?;
    let mut pagination_plans = BTreeMap::new();
    let mut pagination_warnings = Vec::new();
    for (index, operation) in document.operations.iter().enumerate() {
        match pagination::plan(&document, operation)? {
            pagination::Planning::Absent => {}
            pagination::Planning::Emitted(plan) => {
                pagination_plans.insert(index, plan);
            }
            pagination::Planning::Lossy { pointer, reason } => {
                let subject = format!("operations.{}", operation_subject(operation));
                coverage.exact.remove(&subject);
                coverage.opaque.remove(&subject);
                coverage.lossy.insert(subject.clone());
                pagination_warnings.push(format!(
                    "TypeScript cannot generate a pagination iterator at {subject}, because the \
                     items pointer {} does not resolve to an array: {reason}",
                    crate::render::string(&pointer)
                ));
            }
        }
    }
    let type_scan = scan_type_references(&document);
    if let Some(path) = type_scan.fractional_constraint_paths.first() {
        return Err(EmitError(format!(
            "the TypeScript emitter cannot enforce fractional integer constraint {path}"
        )));
    }
    if mode == EmitMode::Strict && !type_scan.decimal_paths.is_empty() {
        return Err(EmitError(format!(
            "the TypeScript emitter cannot read a decimal that crosses the wire as a number at {}, because a JSON number rounds the digits the contract declares",
            type_scan.decimal_paths.join(", ")
        )));
    }
    for subject in subjects_reaching_a_decimal(&type_scan, &document) {
        if coverage.exact.remove(&subject) || coverage.opaque.remove(&subject) {
            coverage.lossy.insert(subject);
        }
    }
    let decimal_paths = type_scan.decimal_paths;
    let opaque_paths = type_scan.opaque_paths;
    let wide_integers = type_scan.wide_paths;
    let date_only = options.date_mode == DateMode::Date && !type_scan.date_paths.is_empty();
    if options.field_case == FieldCase::Camel {
        camel_field_keys(&document)?;
    }
    let names = type_names(&document, &options);
    let preserve_wide_integers = mode == EmitMode::Strict && !wide_integers.is_empty();
    let mut files = vec![
        GeneratedFile {
            path: "types.ts".into(),
            content: types::emit(&document, &names, &options, date_only)?,
        },
        GeneratedFile {
            path: "errors.ts".into(),
            content: errors::error_types(options.errors),
        },
    ];
    let mut validation = validation::Backend::new(&document, &names, &options)?;
    let client_files = client::emit(
        &document,
        &names,
        &options,
        preserve_wide_integers,
        &mut validation,
        &pagination_plans,
    )?;
    let validation_coverage = validation.coverage(&document)?;
    if let Some(file) = validation.render()? {
        files.push(file);
    }
    let module = validation
        .capabilities()
        .map(|capabilities| capabilities.module);
    files.extend(runtime::files(options.runtime));
    files.extend(client_files);
    files.push(client::index(module));
    let files = files;
    strict_coverage_error(mode, &coverage)?;
    strict_validation_error(mode, options.validation.backend, &validation_coverage)?;
    let mut warnings = extensions::warnings(&document).map_err(EmitError)?;
    if mode == EmitMode::Lossy {
        if !wide_integers.is_empty() {
            warnings.push(format!(
                "TypeScript emits wide numeric integers as number at {}",
                wide_integers.join(", ")
            ));
        }
        if !decimal_paths.is_empty() {
            warnings.push(format!(
                "TypeScript emits decimals that cross the wire as numbers at {}",
                decimal_paths.join(", ")
            ));
        }
        if !opaque_paths.is_empty() {
            warnings.push(format!(
                "TypeScript emits opaque values as unknown at {}",
                opaque_paths.join(", ")
            ));
        }
        warnings.extend(pagination_warnings);
        warnings.extend(validation_warnings(
            options.validation.backend,
            &validation_coverage,
        ));
    }
    Ok(EmitOutput {
        files,
        warnings,
        coverage,
        validation_coverage,
    })
}

fn media_codec_paths(document: &Document) -> Vec<String> {
    let mut paths = Vec::new();
    for (index, error) in document.common_errors.iter().enumerate() {
        if let Some(codec) = error
            .media
            .as_ref()
            .and_then(|media| media.codec.as_deref())
        {
            paths.push(format!("common_errors[{index}] ({codec})"));
        }
    }
    for operation in &document.operations {
        let owner = format!("operations.{}", pons_ir::operation_path(&operation.id));
        if let Some(codec) = operation
            .body
            .as_ref()
            .and_then(|body| body.media.as_ref())
            .and_then(|media| media.codec.as_deref())
        {
            paths.push(format!("{owner}.body ({codec})"));
        }
        for (index, response) in operation.responses.iter().enumerate() {
            if let Some(codec) = response
                .media
                .as_ref()
                .and_then(|media| media.codec.as_deref())
            {
                paths.push(format!("{owner}.responses[{index}] ({codec})"));
            }
        }
        for (index, error) in operation.errors.iter().enumerate() {
            if let Some(codec) = error
                .media
                .as_ref()
                .and_then(|media| media.codec.as_deref())
            {
                paths.push(format!("{owner}.errors[{index}] ({codec})"));
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests;
