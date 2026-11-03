mod support;

use pons_emit_ts::{
    EmitMode, ErrorStrategy, Runtime, TsOptions, ValidationBackend, ValidationOptions,
};
use pons_ir::Document;
use serde_json::json;
use support::runtime::{
    emitted_files as emitted, fixture, generated_file as file, kitchen_sink,
    lossy_options as zod_options, run_zod as run_bun, run_zod_with_runtime as run_bun_with,
    zod_files as zod,
};

#[test]
fn a_schema_admits_what_the_wire_validator_admits_and_refuses_the_rest() {
    let document = kitchen_sink();
    let files = zod(&document, zod_options());
    let harness = r#"
import {
  validateAlias, validateBeside, validateChoices, validateGathered, validateLoose, validateMarked, validateScalars,
  validateSealed, validateShapes, validateSpreads, validateTree, validateTyped, validateWhole,
} from "./validators.ts";

const admits = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, check: (value: unknown) => unknown, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};

admits("a string enum", validateChoices, "one");
refuses("an unknown string enum", validateChoices, "three");
refuses("a string enum over a number", validateChoices, 1);

admits("a bare external variant", validateSpreads, "bare");
admits("a bare external variant as an object", validateSpreads, { bare: null });
admits("a bare external variant carrying anything", validateSpreads, { bare: 7 });
admits("an external payload", validateSpreads, { held: 3 });
refuses("two external variants", validateSpreads, { bare: null, held: 3 });
refuses("an unknown external variant", validateSpreads, { ghost: null });
refuses("an external payload of the wrong type", validateSpreads, { held: "3" });

admits("a unit internal variant", validateMarked, { kind: "none" });
admits("an internal unit variant carrying an extra key", validateMarked, { kind: "none", at: 1 });
admits("an internal payload", validateMarked, { kind: "some", at: 1 });
refuses("an internal payload without its field", validateMarked, { kind: "some" });
refuses("an unknown internal variant", validateMarked, { kind: "ghost" });
refuses("an internal variant without its tag", validateMarked, { at: 1 });
refuses("an internal payload out of range", validateMarked, { kind: "some", at: 2 ** 33 });

admits("a unit adjacent variant", validateBeside, { kind: "none" });
admits("an adjacent unit variant carrying an extra key", validateBeside, { kind: "none", payload: 1 });
admits("an adjacent payload", validateBeside, { kind: "some", payload: true });
refuses("an adjacent payload that is missing", validateBeside, { kind: "some" });
refuses("an adjacent payload of the wrong type", validateBeside, { kind: "some", payload: 1 });
refuses("an unknown adjacent variant", validateBeside, { kind: "ghost", payload: true });

admits("an untagged unit", validateShapes, null);
admits("an untagged newtype", validateShapes, 3);
admits("an untagged struct", validateShapes, { name: "a" });
admits("a value two untagged variants fit", validateShapes, { name: "a", extra: "b" });
refuses("a value no untagged variant fits", validateShapes, "a");
refuses("an untagged struct without its field", validateShapes, {});

admits("a newtype chain", validateAlias, 3);
refuses("a newtype chain out of range", validateAlias, 2 ** 33);
refuses("a newtype chain over the wrong type", validateAlias, "3");

admits("a recursive value", validateTree, { value: 1, next: { value: 2, next: null } });
refuses("a recursive value missing a field", validateTree, { value: 1, next: {} });
refuses("a recursive value with an unknown key", validateTree, { value: 1, next: null, other: 1 });

admits("a flattened member", validateWhole, { a: "x", b: true });
refuses("a flattened member that is missing", validateWhole, { a: "x" });
refuses("a flattened member of the wrong type", validateWhole, { a: "x", b: 1 });

admits("a read_only member of a flattened output type", validateGathered, { own: "x", id: "y" });
refuses("a flattened output member that is missing", validateGathered, { own: "x" });

admits("rejected openness", validateSealed, { a: "x" });
refuses("an unknown property", validateSealed, { a: "x", b: 1 });
admits("opaque openness", validateLoose, { a: "x", b: { anything: true } });
admits("typed openness", validateTyped, { a: 1, b: 2 });
refuses("a preserved property of the wrong type", validateTyped, { a: 1, b: "2" });
refuses("a preserved property the field does not allow", validateTyped, { a: "1" });

const scalars = {
  blob: new Uint8Array([65]), clock: "12:00:00Z", day: "2024-02-29", double: 1.5,
  exact: "1.50", link: "https://e.test/a", many: [1, 2], maybe: null, moment: "2024-01-01T00:00:00Z",
  money: 1.25, nothing: null, pair: ["a", true], secret: { any: 1 }, shaped: "abc", single: 1.5,
  sized: "abc", small: 5, span: "P1D", table: { a: 1 }, text: "42", uuid: "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  v4: "1.2.3.4", v6: "::1", whatever: 1, who: "a@b.test", wide: 5,
};
admits("every scalar", validateScalars, scalars);
const changed = (change: Record<string, unknown>) => ({ ...scalars, ...change });
for (const [label, change] of [
  ["an integer below its minimum", { small: -10 }],
  ["an integer above its maximum", { small: 10 }],
  ["an integer off its multiple", { small: 4 }],
  ["an integer above its width", { wide: 2 ** 64 }],
  ["a non-canonical integer string", { text: "007" }],
  ["an integer string that is not one", { text: "1.0" }],
  ["a number outside binary32", { single: 1e39 }],
  ["a decimal that is not one", { exact: "1.2.3" }],
  ["a day that is not one", { day: "2024-02-30" }],
  ["a clock with no offset", { clock: "12:00:00" }],
  ["a uuid that is not one", { uuid: "not-a-uuid" }],
  ["a uri that is not one", { link: "not a uri" }],
  ["an email that is not one", { who: "nope" }],
  ["an ipv4 that is not one", { v4: "999.1.1.1" }],
  ["an ipv6 that is not one", { v6: "::x" }],
  ["an instant with no offset", { moment: "2024-01-01T00:00:00" }],
  ["a month the calendar does not have", { moment: "2024-13-01T00:00:00Z" }],
  ["a string that is too short", { sized: "a" }],
  ["a string that is too long", { sized: "abcde" }],
  ["a string the pattern refuses", { shaped: "ABC" }],
  ["bytes that are not bytes", { blob: [65] }],
  ["a null that is not null", { nothing: 0 }],
  ["a nullable that is neither", { maybe: "x" }],
  ["an array with a repeated item", { many: [1, 1] }],
  ["an array item of the wrong type", { many: [1, "2"] }],
  ["a map value of the wrong type", { table: { a: "1" } }],
  ["a tuple of the wrong length", { pair: ["a"] }],
  ["a tuple item of the wrong type", { pair: ["a", 1] }],
  ["a required field that is absent", { sized: undefined }],
  ["a required opaque that is absent", { secret: undefined }],
  ["an unknown property", { ghost: 1 }],
] as Array<[string, Record<string, unknown>]>) {
  refuses(label, validateScalars, changed(change));
}
const sized = (value: string) => changed({ sized: value });
admits("a string of two astral code points", validateScalars, sized("😀😀"));
refuses("a string of one astral code point", validateScalars, sized("😀"));
"#;
    run_bun(&files, harness);
}

#[test]
fn the_four_byte_encodings_reach_one_decoded_schema_and_keep_their_spelling() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {"wire": "array", "ident": ["array"], "ty": {"kind": "bytes", "encoding": "array"}, "presence": "required"},
        {"wire": "base64", "ident": ["base64"], "ty": {"kind": "bytes", "encoding": "base64"}, "presence": "required"},
        {"wire": "base64url", "ident": ["base64url"], "ty": {"kind": "bytes", "encoding": "base64url"}, "presence": "required"},
        {"wire": "hex", "ident": ["hex"], "ty": {"kind": "bytes", "encoding": "hex"}, "presence": "required"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let schemas = file(&files, "validators.ts");
    for encoding in ["array", "base64", "base64url", "hex"] {
        assert!(
            schemas.contains(&format!("\"{encoding}\": z.instanceof(Uint8Array)")),
            "{encoding} decodes to the bytes a schema checks: {schemas}"
        );
    }
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status: 200, headers: { "content-type": "application/json" } }),
});
const login = async (body: unknown) => {
  try {
    return { value: await answering(JSON.stringify(body)).auth.login({ body: { email: "person@example.test" } }) };
  } catch (failure) {
    return { failure };
  }
};
const body = { array: [65], base64: "QQ==", base64url: "QQ", hex: "41" };
const decoded: any = await login(body);
if (decoded.failure !== undefined) throw new Error(`four encodings were refused: ${JSON.stringify(decoded.failure)}`);
for (const encoding of ["array", "base64", "base64url", "hex"]) {
  const bytes = decoded.value[encoding];
  if (!(bytes instanceof Uint8Array) || bytes.length !== 1 || bytes[0] !== 65) {
    throw new Error(`${encoding} did not decode to its byte: ${bytes}`);
  }
}
for (const [label, broken] of [
  ["an array that is not bytes", { ...body, array: ["65"] }],
  ["base64 that is not base64", { ...body, base64: "!!!" }],
  ["base64url that is not base64url", { ...body, base64url: "???" }],
  ["hex that is not hex", { ...body, hex: "zz" }],
] as Array<[string, Record<string, unknown>]>) {
  const refused = await login(broken);
  if (refused.failure?._tag !== "DecodeError") {
    throw new Error(`${label} was admitted: ${JSON.stringify(refused)}`);
  }
  if (refused.failure.message.includes("the response does not match the contract")) {
    throw new Error(`${label} is the wire validator's to refuse: ${refused.failure.message}`);
  }
}
"#;
    run_bun(&files, harness);
}

#[test]
fn a_streamed_response_checks_its_items_not_its_setup_body() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = zod(&document, TsOptions::default());
    let events = file(&files, "operations/events.ts");
    assert!(
        events.contains("schema: undefined, stream: \"sse\""),
        "the setup response has no body to check: {events}"
    );
    assert!(
        events.contains(
            "events: [{ event: \"user\", schema: {\"ref\":\"User\"}, validate: validateUser }]"
        ),
        "each event carries its own check: {events}"
    );
    assert!(
        events.contains(
            "headers: [{ name: \"X-Trace-ID\", required: true, schema: {\"kind\":\"string\"}, validate: validateInline1 }]"
        ),
        "a declared stream header carries one too: {events}"
    );
    let auth = file(&files, "operations/auth.ts");
    assert!(
        auth.contains("validate: validateUser"),
        "the operation beside it does carry one: {auth}"
    );
}

#[test]
fn a_response_that_breaks_its_schema_is_the_failure_a_broken_body_already_is() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    {
        let files = zod(&document, TsOptions::default());
        let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string, status = 200) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status, headers: { "content-type": "application/json" } }),
});
const login = async (client: any) => {
  try {
    return { value: await client.auth.login({ body: { email: "person@example.test" } }) };
  } catch (failure) {
    return { failure };
  }
};

const broken = await login(answering(JSON.stringify({ id: 1 })));
const malformed = await login(answering(JSON.stringify({})));
const ok = await login(answering(JSON.stringify({ id: "1" })));
if (ok.failure !== undefined) throw new Error(`a good body was refused: ${JSON.stringify(ok.failure)}`);
if (ok.value.id !== "1") throw new Error("the good body did not come back");

const shape = (answer: any) => answer.failure?._tag ?? "ok";
if (shape(broken) !== "DecodeError") throw new Error(`a schema breach is not a protocol failure: ${JSON.stringify(broken)}`);
if (shape(malformed) !== "DecodeError") throw new Error(`a malformed body is not a protocol failure: ${JSON.stringify(malformed)}`);
if (typeof (broken.failure ?? malformed.failure).message !== "string") {
  throw new Error(`a protocol failure names what it saw: ${JSON.stringify(broken)}`);
}
if (broken.failure.message === malformed.failure.message) {
  throw new Error(`the two failures say the same thing: ${broken.failure.message}`);
}
if (!malformed.failure.message.includes("missing required field id")) {
  throw new Error(`the wire validator answers first: ${malformed.failure.message}`);
}
if (broken.failure.message.includes("the response does not match the contract")) {
  throw new Error(`the wire validator already refused this body: ${broken.failure.message}`);
}

const declared = await login(answering(JSON.stringify({ code: "invalid" }), 418));
if (declared.failure._tag !== "ApiError" || declared.failure.status !== 418) {
  throw new Error(`a declared error status is still an http failure: ${JSON.stringify(declared)}`);
}
if (declared.failure.body.code !== "invalid") {
  throw new Error(`the declared error body is checked too: ${JSON.stringify(declared)}`);
}
const unexpected = await login(answering(JSON.stringify({ id: "1" }), 500));
if (unexpected.failure._tag !== "UnexpectedStatusError" || unexpected.failure.status !== 500) {
  throw new Error(`a status no list declares is an unexpected_http: ${JSON.stringify(unexpected)}`);
}
"#;
        run_bun(&files, harness);
    }
    for errors in [ErrorStrategy::Neverthrow, ErrorStrategy::Throw] {
        let files = emitted(
            &document,
            TsOptions {
                validation: ValidationOptions::new(ValidationBackend::Zod),
                errors,
                ..TsOptions::default()
            },
        );
        let errors_file = file(&files, "errors.ts");
        for kind in [
            "NetworkError",
            "TimeoutError",
            "AbortedError",
            "AuthAdapterError",
            "AuthSourceError",
            "OpaqueCodecError",
            "TransportHookError",
            "DecodeError",
            "UnexpectedStatusError",
            "AuthUnavailableError",
        ] {
            assert!(
                errors_file.contains(kind),
                "{kind} is still in the closed union under {errors:?}: {errors_file}"
            );
        }
        assert!(
            !errors_file.contains("schema"),
            "a schema breach joins the union as a DecodeError, not as a new kind: {errors_file}"
        );
    }
}

#[test]
fn the_schema_checks_the_formats_the_wire_validator_leaves_open() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {"wire": "id", "ident": ["id"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status: 200, headers: { "content-type": "application/json" } }),
});
const good = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
const accepted: any = await answering(JSON.stringify({ id: good })).auth.login({ body: { email: "person@example.test" } });
if (accepted.id !== good) throw new Error(`a uuid the contract allows was refused: ${JSON.stringify(accepted)}`);

let refused: any;
try {
  await answering(JSON.stringify({ id: "not-a-uuid" })).auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  refused = cause;
}
if (refused?._tag !== "DecodeError") throw new Error(`the schema did not answer: ${JSON.stringify(refused)}`);
if (!refused.message.includes("the response does not match the contract")) {
  throw new Error(`the failure names the schema: ${refused.message}`);
}
if (!refused.message.includes("id")) throw new Error(`the failure names the path: ${refused.message}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn the_format_corpus_accepts_and_refuses_what_the_builtin_does() {
    let mut raw = fixture();
    raw["types"] = json!({
        "Formats": {
            "kind": "struct", "name": ["formats"], "direction": "output", "openness": "reject",
            "fields": [
                {"wire": "moment", "ident": ["moment"], "ty": {"kind": "string", "format": "date-time"}, "presence": "required"},
                {"wire": "day", "ident": ["day"], "ty": {"kind": "string", "format": "date"}, "presence": "required"},
                {"wire": "clock", "ident": ["clock"], "ty": {"kind": "string", "format": "time"}, "presence": "required"},
                {"wire": "span", "ident": ["span"], "ty": {"kind": "string", "format": "duration"}, "presence": "required"},
                {"wire": "uuid", "ident": ["uuid"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"},
                {"wire": "link", "ident": ["link"], "ty": {"kind": "string", "format": "uri"}, "presence": "required"},
                {"wire": "who", "ident": ["who"], "ty": {"kind": "string", "format": "email"}, "presence": "required"},
                {"wire": "v4", "ident": ["v4"], "ty": {"kind": "string", "format": "ipv4"}, "presence": "required"},
                {"wire": "v6", "ident": ["v6"], "ty": {"kind": "string", "format": "ipv6"}, "presence": "required"}
            ]
        }
    });
    raw["operations"] = json!([{
        "id": [["formats"], ["read"]],
        "method": "GET",
        "path": [{"lit": "formats"}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"ref": "Formats"}}],
        "errors": []
    }]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let schemas = file(&files, "validators.ts");
    for expected in [
        "const DATE = /^(\\d{4})-(\\d{2})-(\\d{2})$/;",
        "const TIME = /^(\\d{2}):(\\d{2}):(\\d{2})(?:\\.\\d+)?(?:[Zz]|([+-])(\\d{2}):(\\d{2}))$/;",
        "const DATE_TIME = /^(\\d{4})-(\\d{2})-(\\d{2})[Tt](\\d{2}):(\\d{2}):(\\d{2})(?:\\.\\d{1,3})?(?:[Zz]|([+-])(\\d{2}):(\\d{2}))$/;",
        "const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;",
        "const EMAIL = ",
        "const DURATION = ",
        "const IPV4 = ",
        "const IPV6 = ",
        "const instant = () =>",
        "const day = () =>",
        "const clock = () =>",
        "const uri = () =>",
    ] {
        assert!(
            schemas.contains(expected),
            "the format corpus emits {expected}: {schemas}"
        );
    }
    let harness = r#"
import { validateContract } from "./runtime.ts";
import { validateFormats } from "./validators.ts";

const base = {
  moment: "2024-02-29T23:59:59Z",
  day: "2024-02-29",
  clock: "23:59:59.999+05:30",
  span: "P1Y2M3DT4H5M6.789S",
  uuid: "12345678-1234-0234-0234-123456789abc",
  link: "https://example.test/a",
  who: "first.last+tag@sub.example.test",
  v4: "255.255.255.255",
  v6: "2001:db8::8a2e:370:7334",
};
const builtin = (value: unknown) => validateContract(value, { ref: "Formats" });
const zod = (value: unknown) => validateFormats(value);

const accepts: Record<string, string[]> = {
  moment: ["2024-02-29T23:59:59Z", "2024-01-01T00:00:00.123+02:00", "2024-01-01t00:00:00z", "2000-02-29T12:00:00-05:00"],
  day: ["2024-02-29", "2000-02-29", "2023-12-31"],
  clock: ["00:00:00Z", "23:59:59.999+05:30", "12:00:00z"],
  span: ["P1D", "P1W", "PT1H", "P1Y2M3DT4H5M6.789S", "P1Y", "PT0S"],
  uuid: ["6ba7b810-9dad-11d1-80b4-00c04fd430c8", "12345678-1234-0234-0234-123456789abc", "6BA7B810-9DAD-11D1-80B4-00C04FD430C8"],
  link: ["https://example.test/a", "mailto:a@b.co", "urn:isbn:0451450523", "http://example.test"],
  who: ["a@b.co", "first.last+tag@sub.example.test", "o'hara@example.com"],
  v4: ["0.0.0.0", "255.255.255.255", "1.2.3.4", "99.99.99.99"],
  v6: ["::1", "::", "2001:db8::8a2e:370:7334", "fe80::1", "1:2:3:4:5:6:7:8"],
};
const refuses: Record<string, string[]> = {
  moment: ["2024-02-30T00:00:00Z", "2024-01-01T24:00:00Z", "2024-01-01T00:00:00", "2024-01-01T00:00:00.1234Z", "2024-13-01T00:00:00Z", "2024-01-01 00:00:00Z"],
  day: ["2024-02-30", "2023-02-29", "2024-13-01", "2024-1-01", "2024-01-01T00:00:00Z"],
  clock: ["24:00:00Z", "12:60:00Z", "12:00:60Z", "12:00:00", "12:00:00+24:00", "12:00:00+00:60"],
  span: ["P", "P1W2D", "P1Y2W", "1D", "P1H", "PT", "P1DT"],
  uuid: ["not-a-uuid", "6ba7b810-9dad-11d1-80b4-00c04fd430c", "6ba7b8109dad11d180b400c04fd430c8"],
  link: ["not a uri", "/relative", "example.test", "https://exa mple.test", ""],
  who: ["nope", "a@b", "a@b.c", "a b@c.co", "@b.co", "a@.co"],
  v4: ["256.1.1.1", "999.1.1.1", "1.2.3", "1.2.3.4.5", "01.2.3.4"],
  v6: ["::x", "2001:db8:::1", "1:2:3:4:5:6:7:8:9", "1:2:3:4:5:6:7", ":::"],
};

for (const [key, values] of Object.entries(accepts)) {
  for (const value of values) {
    const candidate = { ...base, [key]: value };
    const reference = builtin(candidate);
    if (reference !== null) throw new Error(`the builtin refuses ${key} ${JSON.stringify(value)}: ${JSON.stringify(reference)}`);
    const issue = zod(candidate);
    if (issue !== null) throw new Error(`zod refuses ${key} ${JSON.stringify(value)}: ${JSON.stringify(issue)}`);
  }
}
for (const [key, values] of Object.entries(refuses)) {
  for (const value of values) {
    const candidate = { ...base, [key]: value };
    const reference = builtin(candidate);
    if (reference === null) throw new Error(`the builtin admits ${key} ${JSON.stringify(value)}`);
    const issue = zod(candidate);
    if (issue === null) throw new Error(`zod admits ${key} ${JSON.stringify(value)}`);
  }
}
"#;
    run_bun(&files, harness);
}

#[test]
fn the_wire_validator_still_answers_a_request_it_refuses() {
    let mut raw = fixture();
    raw["types"]["LoginRequest"] = json!({
        "kind": "struct", "name": ["login", "request"], "direction": "input", "openness": "reject",
        "fields": [
            {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
            {"wire": "id", "ident": ["id"], "ty": {"kind": "string"}, "presence": "required", "access": "read_only", "default": "filled"}
        ]
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(&document, TsOptions::default());
    let harness = r#"
import { createClient } from "./client.ts";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => { throw new Error("the request should not be sent"); },
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test", id: "1" } } as any);
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "RequestError") throw new Error(`expected a protocol failure: ${JSON.stringify(failure)}`);
if (!failure.message.includes("is read_only and cannot cross the encode direction")) {
  throw new Error(`the wire validator answers the request: ${failure.message}`);
}
"#;
    run_bun(&files, harness);
}

#[test]
fn an_untagged_enum_takes_the_first_variant_that_fits() {
    let mut raw = fixture();
    raw["types"]["Shapes"] = json!({
        "kind": "enum", "name": ["shapes"], "direction": "output", "repr": {"kind": "untagged"},
        "variants": [
            {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}},
            {"wire": "named", "ident": ["named"], "payload": {"kind": "struct", "fields": [
                {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"}
            ]}},
            {"wire": "full", "ident": ["full"], "payload": {"kind": "struct", "fields": [
                {"wire": "name", "ident": ["name"], "ty": {"kind": "string"}, "presence": "required"},
                {"wire": "extra", "ident": ["extra"], "ty": {"kind": "string"}, "presence": "required"}
            ]}},
            {"wire": "nothing", "ident": ["nothing"], "payload": {"kind": "unit"}}
        ]
    });
    raw["operations"][0]["responses"][0]["body"] = json!({"ref": "Shapes", "variant": "count"});
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(
        &document,
        TsOptions {
            mode: EmitMode::Lossy,
            ..TsOptions::default()
        },
    );
    let schemas = file(&files, "validators.ts");
    assert!(
        schemas.contains("const ShapesSchema = z.union([ShapesCountSchema, ShapesFullSchema, ShapesNamedSchema, ShapesNothingSchema]);"),
        "the union keeps the order the canonical document declares: {schemas}"
    );
    let harness = r#"
import { createClient } from "./client.ts";
import { validateShapes, validateShapesCount } from "./validators.ts";

const check = validateShapes;
const admits = (label: string, value: unknown) => {
  const found = check(value);
  if (found !== null) throw new Error(`${label} was refused: ${JSON.stringify(found)}`);
};
const refuses = (label: string, value: unknown) => {
  if (check(value) === null) throw new Error(`${label} was admitted`);
};
admits("a count", 3);
admits("a struct", { name: "a" });
admits("a value two variants fit", { name: "a", extra: "b" });
admits("a unit", null);
refuses("a struct without its field", {});
refuses("a value no variant fits", "a");
refuses("a count out of range", 2 ** 33);
if (validateShapesCount(2 ** 33) === null) throw new Error("the narrowed validator bounds the count");

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("3", { status: 200, headers: { "content-type": "application/json" } }),
});
const value: any = await client.auth.login({ body: { email: "person@example.test" } });
if (value !== 3) throw new Error(`the narrowed response is checked: ${JSON.stringify(value)}`);
"#;
    run_bun(&files, harness);
}

#[test]
fn the_packaged_runtime_carries_the_hook_the_option_uses() {
    let mut raw = fixture();
    raw["types"]["User"]["fields"] = json!([
        {"wire": "id", "ident": ["id"], "ty": {"kind": "string", "format": "uuid"}, "presence": "required"}
    ]);
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = zod(
        &document,
        TsOptions {
            runtime: Runtime::Package,
            ..TsOptions::default()
        },
    );
    assert!(
        !files
            .iter()
            .any(|file| file.path == "runtime.ts" || file.path.starts_with("runtime/")),
        "the package supplies the runtime, so the emitter writes none of it"
    );
    assert!(
        file(&files, "client.ts").contains("\"@pons/runtime\""),
        "the package is the runtime the client imports"
    );
    let harness = r#"
import { createClient } from "./client.ts";

const answering = (body: string) => createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(body, { status: 200, headers: { "content-type": "application/json" } }),
});
const good = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
let accepted: any;
try {
  accepted = await answering(JSON.stringify({ id: good })).auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  throw new Error(`a good body was refused: ${JSON.stringify(cause)}`);
}
if (accepted.id !== good) throw new Error(`a good body did not come back: ${JSON.stringify(accepted)}`);
let refused: any;
try {
  await answering(JSON.stringify({ id: "not-a-uuid" })).auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  refused = cause;
}
if (refused?._tag !== "DecodeError") throw new Error(`the packaged runtime did not answer: ${JSON.stringify(refused)}`);
"#;
    run_bun_with(&files, harness, Runtime::Package);
}
