mod diff;
mod document;
mod media;
mod operation;
mod types;

pub use diff::{Change, ChangeLevel, Diff};
pub use document::{Document, Info, ValidationError};
pub use media::is_valid_media_type;
pub use operation::{
    ApiKeyLocation, AuthRequirement, AuthScheme, Body, ContentKind, CredentialLocator,
    CredentialProvision, ErrorContent, ErrorResponse, ErrorStatus, HeaderParameter, Method,
    OAuthFlow, OAuthFlowKind, Operation, OperationSemantics, Origin, PathSegment, Response,
    ResponseHeader, ResponseStatus, StreamDescription, StreamKind, StreamMessage,
};
pub use types::{
    Access, ByteEncoding, DecimalWire, Diagnostic, DiagnosticSeverity, Direction, Doc, EnumRepr,
    Field, FieldDefault, FieldFormat, InlineType, IntegerWire, Name, NamedRef, ObjectOpenness,
    Presence, TypeDef, TypeKind, TypeRef, Variant, VariantPayload,
};

pub const IR_VERSION: &str = "0.2";
