mod support;

use pons_ir::Document;

use support::type_check::{files_with, fixture, type_checks_beside, type_checks_with_packages};

#[test]
fn a_client_that_checks_its_responses_against_zod_compiles_beside_zod() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Zod,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        assert!(
            files.iter().any(|file| file.path == "validators.ts"),
            "the option writes the validators: {:?}",
            files.iter().map(|file| &file.path).collect::<Vec<_>>()
        );
        type_checks_beside(
            &files,
            r#"
import { createClient } from "./client.js";
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const client = createClient({ baseUrl: "https://api.example.test" });
export const loggedIn = await client.auth.login({ body: { email: "person@example.test" } });
export const id = loggedIn.id;
export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            true,
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_valibot_compiles_beside_valibot() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Valibot,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["valibot"],
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_superstruct_compiles_beside_superstruct() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Superstruct,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["superstruct"],
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_typebox_compiles_beside_typebox() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Typebox,
                ),
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["typebox"],
        );
    }
}

#[test]
fn a_client_that_checks_its_responses_against_yup_compiles_beside_yup() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    for packaged in [false, true] {
        let files = files_with(
            &document,
            pons_emit_ts::TsOptions {
                validation: pons_emit_ts::ValidationOptions::new(
                    pons_emit_ts::ValidationBackend::Yup,
                ),
                mode: pons_emit_ts::EmitMode::Lossy,
                runtime: if packaged {
                    pons_emit_ts::Runtime::Package
                } else {
                    pons_emit_ts::Runtime::Vendor
                },
                ..pons_emit_ts::TsOptions::default()
            },
        );
        type_checks_with_packages(
            &files,
            r#"
import { validateLoginErrorInvalid, validateUser } from "./validators.js";

export const user = validateUser({ id: "1" });
export const invalid = validateLoginErrorInvalid({ code: "invalid" });
// @ts-expect-error a validator takes one argument and returns a verdict
export const wrong = validateUser({ id: "1" }, { id: "1" });
"#,
            packaged,
            &["yup"],
        );
    }
}

#[test]
fn a_validator_over_every_format_compiles_beside_zod() {
    let mut raw = fixture();
    raw["types"]["User"] = serde_json::json!({
        "kind": "struct", "name": ["user"], "direction": "output", "openness": "reject",
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
    });
    raw["operations"][0]["provides"] = serde_json::Value::Null;
    let document: Document = serde_json::from_value(raw).unwrap();
    let files = files_with(
        &document,
        pons_emit_ts::TsOptions {
            validation: pons_emit_ts::ValidationOptions::new(pons_emit_ts::ValidationBackend::Zod),
            ..pons_emit_ts::TsOptions::default()
        },
    );
    type_checks_beside(
        &files,
        r#"
import { validateUser } from "./validators.js";

export const issue = validateUser({
  moment: "2024-01-01T00:00:00Z",
  day: "2024-01-01",
  clock: "00:00:00Z",
  span: "P1D",
  uuid: "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  link: "https://example.test",
  who: "a@b.co",
  v4: "1.2.3.4",
  v6: "::1",
});
"#,
        false,
        true,
    );
}
