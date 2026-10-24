use pons_ir::Document;
use serde_json::{Value, json};

use super::{emitted, fixture};

fn documented() -> Document {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["User"]["doc"] = json!({
        "summary": "A person",
        "description": "As the contract records them."
    });
    raw["types"]["User"]["fields"][1]["doc"] =
        json!({"summary": "How they would like to be called."});
    raw["types"]["LoginError"]["variants"][0]["doc"] = json!({
        "summary": "The credentials were refused.",
        "deprecated": true
    });
    raw["operations"][0]["doc"] = json!({
        "summary": "Exchange a password for a session.",
        "description": "The session token arrives in the body."
    });
    raw["operations"][0]["errors"][0]["doc"] = json!({"summary": "The credentials did not match."});
    serde_json::from_value(raw).unwrap()
}

#[test]
fn a_documented_type_carries_its_summary_and_its_description() {
    let (types, ..) = emitted(&documented());
    assert!(
        types.contains("/** A person As the contract records them. */\nexport type User = {"),
        "{types}"
    );
}

#[test]
fn a_documented_struct_field_carries_its_documentation() {
    let (types, ..) = emitted(&documented());
    assert!(
            types.contains(
                "export type User = {\n  \"id\": string;\n  /** How they would like to be called. */\n  \"nickname\"?: string | null;\n  [key: string]: unknown;\n};"
            ),
            "{types}"
        );
}

#[test]
fn a_documented_enum_variant_carries_its_documentation() {
    let (types, ..) = emitted(&documented());
    assert!(
            types.contains(
                "export type LoginError = /** The credentials were refused. @deprecated */\n  { \"code\": \"invalid\" }\n  | { \"code\": \"locked\" } & { \"retry_after\": number; [key: string]: unknown };"
            ),
            "{types}"
        );
}

#[test]
fn a_documented_operation_carries_its_documentation() {
    let (_, _, operations) = emitted(&documented());
    assert!(
            operations["auth"].contains(
                "/** Exchange a password for a session. The session token arrives in the body. */\n    login: (args: AuthLoginArgs, options?: CallOptions) =>"
            ),
            "{}",
            operations["auth"]
        );
}

#[test]
fn a_documented_error_response_carries_its_documentation() {
    let (_, _, operations) = emitted(&documented());
    assert!(
        operations["auth"].contains("export type AuthLoginError = ClientFailure")
            && operations["auth"].contains("ApiError<number, Extract<Types.LoginError"),
        "{}",
        operations["auth"]
    );
}

#[test]
fn a_summary_keeps_its_comment_whole() {
    let mut raw: Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["User"]["doc"] = json!({
        "summary": "Ends the comment early */ and then\ncarries on",
        "description": "Second line."
    });
    let (types, client, _) = emitted(&serde_json::from_value(raw).unwrap());
    for source in [&types, &client] {
        assert_eq!(
            source.matches("/**").count(),
            source.matches("*/").count(),
            "every comment is closed:\n{source}"
        );
    }
    assert!(
        types.contains("/** Ends the comment early *\\/ and then carries on Second line. */"),
        "{types}"
    );
    assert!(!types.contains("and then\ncarries on"), "{types}");
}

#[test]
fn a_document_that_documents_nothing_emits_the_bytes_it_emitted_before() {
    let (types, client, _) = emitted(&fixture());
    assert!(
        types.starts_with(TYPES_WITHOUT_DOCS),
        "the declarations carry no comment the contract did not write:\n{types}"
    );
    assert!(
            types.contains(
                "export const typeDefinitions: Record<string, any> = Object.assign(Object.create(null), JSON.parse("
            ),
            "the runtime reads the wire shape of the same declarations:\n{types}"
        );
    assert_eq!(fnv1a(&client), CLIENT_WITHOUT_DOCS);
    let handwritten = client
        .match_indices("/**")
        .map(|(at, _)| &client[at..at + client[at..].find("*/").unwrap() + 2])
        .collect::<Vec<_>>();
    assert_eq!(
        handwritten,
        vec![
            "/** Scope and lifetime captured with a credential. `expiresAt` is a Unix \
                 millisecond timestamp; `host`, `path`, `secure`, and `origin` bind the value to \
                 the response that issued it. */",
            "/** A store returns a `{ value, ...metadata }` record, never a bare string, and \
                 must persist the metadata it is given in `set` so that expiry and origin scope \
                 survive a new client. A record with a non-string `value` or a metadata field of \
                 the wrong type is rejected. */",
        ],
        "the emitter documents nothing the contract did not document"
    );
}

fn fnv1a(source: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

const TYPES_WITHOUT_DOCS: &str = concat!(
    "export type EventQuery = { \"limit\"?: number };\n",
    "\n",
    "export type LoginError = { \"code\": \"invalid\" } | { \"code\": \"locked\" } & ",
    "{ \"retry_after\": number; [key: string]: unknown };\n",
    "\n",
    "export type LoginRequest = { \"email\": string; [key: string]: unknown };\n",
    "\n",
    "export type User = { \"id\": string; \"nickname\"?: string | null; ",
    "[key: string]: unknown };\n",
    "\n"
);
const CLIENT_WITHOUT_DOCS: u64 = 0x8e78_b447_958f_a184;
