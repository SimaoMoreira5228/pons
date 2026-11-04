mod support;

use std::fs;

use pons_emit_ts::{
    DateMode, EmitMode, ErrorStrategy, FieldCase, Int64, Namespaces, Newtypes, Runtime, TsOptions,
    ValidationOptions, emit_with_options, emit_with_strategy,
};
use pons_ir::Document;
use support::options::{emitted, file, fixture, paths, run_bun_with};

#[test]
fn the_default_options_emit_the_bytes_the_three_parameters_did() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let default = TsOptions::default();
    for (mode, errors) in [
        (EmitMode::Strict, ErrorStrategy::Neverthrow),
        (EmitMode::Strict, ErrorStrategy::Throw),
        (EmitMode::Lossy, ErrorStrategy::Neverthrow),
        (EmitMode::Lossy, ErrorStrategy::Throw),
    ] {
        assert_eq!(
            emit_with_options(
                &document,
                TsOptions {
                    mode,
                    errors,
                    ..default.clone()
                }
            )
            .unwrap(),
            emit_with_strategy(&document, mode, errors).unwrap(),
            "the default {mode:?}/{errors:?} is the output the two parameters produced"
        );
    }
    assert_eq!(
        default,
        TsOptions {
            mode: EmitMode::Strict,
            errors: ErrorStrategy::Neverthrow,
            namespaces: Namespaces::Nested,
            field_case: FieldCase::Wire,
            newtypes: Newtypes::Alias,
            int64: Int64::Bigint,
            date_mode: DateMode::String,
            runtime: Runtime::Vendor,
            validation: ValidationOptions::default(),
        },
        "every option has one default and the default is the output of today"
    );
}

#[test]
fn the_default_options_leave_the_vendored_runtime_the_bytes_it_was() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = emitted(&document, TsOptions::default());
    for module in [
        "runtime/wire.ts",
        "runtime/codec.ts",
        "runtime/validate.ts",
        "runtime/stream.ts",
        "runtime/credentials.ts",
        "runtime/transport.ts",
    ] {
        let source = fs::read_to_string(format!(
            "{}/runtime/{}",
            env!("CARGO_MANIFEST_DIR"),
            module.strip_prefix("runtime/").unwrap()
        ))
        .unwrap();
        assert_eq!(
            file(&files, module),
            source,
            "{module} is the file this repository holds"
        );
    }
    let types = file(&files, "types.ts");
    assert!(
        !types.contains("DateOnly") && types.contains("export type User = { \"id\": string"),
        "the default options add nothing to the type the caller reads:\n{types}"
    );
    assert!(
        types.contains("\\\"carrier\\\":\\\"string\\\""),
        "the default metadata names the carrier the wire already dictates:\n{types}"
    );
    assert!(
        !types.contains("\\\"carrier\\\":\\\"date\\\""),
        "the default date mode leaves the wire string in place:\n{types}"
    );
}

#[test]
fn runtime_package_writes_no_runtime_and_imports_the_package_instead() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let vendored_options = TsOptions {
        errors: ErrorStrategy::Throw,
        ..TsOptions::default()
    };
    let vendor = emitted(&document, vendored_options.clone());
    let package = emitted(
        &document,
        TsOptions {
            runtime: Runtime::Package,
            ..vendored_options
        },
    );
    assert_eq!(
        paths(&vendor),
        [
            "client.ts",
            "errors.ts",
            "index.ts",
            "operations/auth.ts",
            "operations/events.ts",
            "runtime.ts",
            "runtime/codec.ts",
            "runtime/credentials.ts",
            "runtime/index.ts",
            "runtime/pagination.ts",
            "runtime/stream.ts",
            "runtime/transport.ts",
            "runtime/validate.ts",
            "runtime/websocket.ts",
            "runtime/wire.ts",
            "types.ts",
        ]
        .map(String::from)
        .to_vec(),
        "the default writes the runtime"
    );
    assert_eq!(
        paths(&package),
        [
            "client.ts",
            "errors.ts",
            "index.ts",
            "operations/auth.ts",
            "operations/events.ts",
            "types.ts",
        ]
        .map(String::from)
        .to_vec(),
        "the package supplies the runtime, so the emitter writes neither it nor its entry point"
    );
    const MARKER: &str = "// runtime = \"package\": the runtime is the @pons/runtime package beside this file, not the vendored runtime/*.ts the default writes here\n";
    for emitted_file in &package {
        let swapped = file(&vendor, &emitted_file.path)
            .replace("from \"../runtime.js\"", "from \"@pons/runtime\"")
            .replace("from \"./runtime.js\"", "from \"@pons/runtime\"");
        let expected = if emitted_file.path == "client.ts" {
            format!("{MARKER}{swapped}")
        } else {
            swapped
        };
        assert_eq!(
            emitted_file.content, expected,
            "only the runtime import changes, and the client names the mode: {}",
            emitted_file.path
        );
    }
    assert!(
        file(&package, "client.ts").starts_with(MARKER),
        "the file set no longer says which mode wrote it, so the client says it: {}",
        file(&package, "client.ts")
    );
    assert!(
        file(&package, "client.ts").contains(
            "import type { ConfiguredClient, CredentialValue, OpaqueCodecRegistry, WebSocketConstructor } from \
             \"@pons/runtime\";"
        ) && file(&package, "client.ts").contains(
            "export type { OpaqueCodec, OpaqueCodecRegistry, OpaqueWire } from \"@pons/runtime\";"
        ),
        "{}",
        file(&package, "client.ts")
    );
    assert!(
        file(&package, "operations/auth.ts").contains(
            "import { request, requestOrThrow, encodePathSegment, type ConfiguredClient } from \
             \"@pons/runtime\";"
        ),
        "{}",
        file(&package, "operations/auth.ts")
    );
    for emitted_file in &package {
        assert!(
            !emitted_file.content.contains("runtime.js"),
            "{} still names the vendored file: {}",
            emitted_file.path,
            emitted_file.content
        );
    }
    run_bun_with(
        &package,
        r#"
import { createClient } from "./client.js";

const client = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
let failure: any;
try {
  await client.auth.login({ body: { email: "person@example.test" } });
} catch (cause) {
  failure = cause;
}
if (failure?._tag !== "DecodeError" || !failure.issues.some((issue: string) => issue.includes("missing required field id"))) {
  throw new Error(`the wire validator in the package did not refuse it: ${JSON.stringify(failure)}`);
}
"#,
        Runtime::Package,
    );
}
