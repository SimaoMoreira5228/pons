mod diff;
mod document;
mod operation;
mod types;

pub use diff::{Change, ChangeLevel, Diff};
pub use document::{Document, Info, ValidationError};
pub use operation::{
    ApiKeyLocation, AuthRequirement, AuthScheme, Body, ContentKind, CredentialLocator,
    CredentialProvision, ErrorContent, ErrorResponse, ErrorStatus, HeaderParameter, Method,
    OAuthFlow, OAuthFlowKind, Operation, OperationSemantics, Origin, PathSegment, Response,
    ResponseHeader, StreamDescription, StreamKind, StreamMessage,
};
pub use types::{
    Access, ByteEncoding, DecimalWire, Diagnostic, DiagnosticSeverity, Direction, Doc, EnumRepr,
    Field, FieldDefault, FieldFormat, InlineType, IntegerWire, Name, NamedRef, Presence, TypeDef,
    TypeKind, TypeRef, Variant, VariantPayload,
};

pub const IR_VERSION: &str = "0.1";
