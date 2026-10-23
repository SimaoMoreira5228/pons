use crate::ErrorStrategy;

const RESULT: &str = r#"export type ResponseWithHeaders<T, H> = { value: T; headers: H };
export type ResponseWithStatus<T> = { value: T; status: number };

export interface NetworkError { readonly _tag: "NetworkError"; readonly cause: unknown }
export interface TimeoutError { readonly _tag: "TimeoutError"; readonly afterMs: number }
export interface AbortedError { readonly _tag: "AbortedError"; readonly cause?: unknown }
export interface DecodeError {
  readonly _tag: "DecodeError";
  readonly status: number;
  readonly issues: readonly string[];
  readonly message: string;
  readonly body: unknown;
  readonly cause?: unknown;
}
export interface UnexpectedStatusError { readonly _tag: "UnexpectedStatusError"; readonly status: number; readonly body: unknown }
export interface ApiError<S extends number, B, H = Headers> { readonly _tag: "ApiError"; readonly status: S; readonly body: B; readonly headers: H }

export interface AuthUnavailableError { readonly _tag: "AuthUnavailableError"; readonly schemes: readonly string[] }
export interface AuthAdapterError { readonly _tag: "AuthAdapterError"; readonly scheme: string; readonly cause: unknown }
export interface AuthSourceError { readonly _tag: "AuthSourceError"; readonly scheme: string; readonly cause: unknown }
export interface CredentialRedirectError { readonly _tag: "CredentialRedirectError"; readonly schemes: readonly string[] }
export interface RequestError { readonly _tag: "RequestError"; readonly message: string }
export interface RedirectError { readonly _tag: "RedirectError"; readonly message: string }
export interface ConfigurationError { readonly _tag: "ConfigurationError"; readonly message: string }
export interface OpaqueCodecError { readonly _tag: "OpaqueCodecError"; readonly codec: string; readonly phase: "encode" | "decode"; readonly cause: unknown }
export interface TransportHookError { readonly _tag: "TransportHookError"; readonly phase: "request" | "response"; readonly cause: unknown }

export type TransportError = NetworkError | TimeoutError | AbortedError;
export type ProtocolError = DecodeError | UnexpectedStatusError;
export type ClientFailure =
  | TransportError
  | ProtocolError
  | AuthUnavailableError
  | AuthAdapterError
  | AuthSourceError
  | CredentialRedirectError
  | RequestError
  | RedirectError
  | ConfigurationError
  | OpaqueCodecError
  | TransportHookError;

export type PonsError<A extends ApiError<number, unknown, unknown>> = ClientFailure | A;

export interface StreamReadError { readonly _tag: "StreamReadError"; readonly cause: unknown }
export interface StreamUnknownEventError { readonly _tag: "StreamUnknownEventError"; readonly event: string }
export interface StreamDecodeError { readonly _tag: "StreamDecodeError"; readonly event?: string; readonly cause: unknown }
export interface StreamServerError<B = unknown> { readonly _tag: "StreamServerError"; readonly body: B }
export type SseStreamError<B = unknown> = StreamReadError | StreamUnknownEventError | StreamDecodeError | StreamServerError<B>;
export type NdjsonStreamError = StreamReadError | StreamDecodeError;

export interface WebSocketClosedError { readonly _tag: "WebSocketClosedError"; readonly code: number; readonly reason: string; readonly cause?: unknown }
export interface WebSocketSendError { readonly _tag: "WebSocketSendError"; readonly cause: unknown }
export interface WebSocketDecodeError { readonly _tag: "WebSocketDecodeError"; readonly cause: unknown }
export interface WebSocketValidationError { readonly _tag: "WebSocketValidationError"; readonly cause: unknown }
export type WebSocketFailure = WebSocketClosedError | WebSocketSendError | WebSocketDecodeError | WebSocketValidationError;
"#;

pub(super) fn error_types(error_strategy: ErrorStrategy) -> String {
    let mut output = if error_strategy == ErrorStrategy::Throw {
        String::from(
            "type Result<T, E> = { isOk(): boolean; isErr(): boolean; value?: T; error?: E };\nfunction ok<T>(value: T): Result<T, never> { return { isOk: () => true, isErr: () => false, value }; }\nfunction err<E>(error: E): Result<never, E> { return { isOk: () => false, isErr: () => true, error }; }\nexport { ok, err };\nexport type { Result };\n",
        )
    } else {
        String::from(
            "import { err, ok, type Result } from \"neverthrow\";\nexport { err, ok };\nexport type { Result };\n",
        )
    };
    output.push_str(RESULT);
    output
}
