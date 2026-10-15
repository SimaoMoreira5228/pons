mod coverage;
mod diff;
mod document;
mod equivalence;
mod ext;
mod media;
mod operation;
mod pattern;
mod query;
mod reported_coverage;
mod types;
mod value;
mod violation;

pub use coverage::{CoverageLevel, canonical_coverage, coverage_subjects, strict_violations};
pub use diff::{Change, ChangeLevel, Diff, Validity};
pub use document::{DecodeError, Document, Info, ValidationError, decode};
pub use equivalence::{EquivalenceError, equivalent};
pub use ext::RESERVED_PREFIX as EXT_RESERVED_PREFIX;
pub use media::{is_valid_media_type, media_type_matches};
pub use operation::{
    ApiKeyLocation, AuthRequirement, AuthScheme, Body, ContentKind, CredentialLocator,
    CredentialProvision, ErrorContent, ErrorResponse, ErrorStatus, HeaderParameter, Method,
    OAuthFlow, OAuthFlowKind, Operation, OperationSemantics, Origin, PathSegment, Response,
    ResponseHeader, ResponseStatus, StreamDescription, StreamKind, StreamMessage, operation_path,
};
pub use query::{QueryRejection, query_fields};
pub use reported_coverage::{
    Coverage, ValidationCoverage, coverage_for_document, operation_subject,
    strict_violation_message, type_subject,
};
pub use types::{
    Access, ByteEncoding, DecimalWire, Diagnostic, DiagnosticSeverity, Direction, Doc, EnumRepr,
    Field, FieldDefault, FieldFormat, InlineType, IntegerWire, Name, NamedRef, ObjectOpenness,
    OpaqueWire, Presence, TypeDef, TypeKind, TypeRef, Variant, VariantPayload,
};
pub use value::validate_value;
pub use violation::{IntegerProblem, Violation};

pub const IR_VERSION: &str = "0.2";
