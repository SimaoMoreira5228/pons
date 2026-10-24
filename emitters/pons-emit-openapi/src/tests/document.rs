use std::collections::BTreeMap;

use pons_ir::{
    AuthScheme, ByteEncoding, ContentKind, Direction, ErrorStatus, InlineType, NamedRef, OAuthFlow,
    OAuthFlowKind, ObjectOpenness, PathSegment, Presence, ResponseHeader, ResponseStatus,
    StreamKind, TypeKind, TypeRef,
};
use serde_json::{Value, json};

use super::{fixture, named_type, string};

#[test]
fn emits_openapi_31_paths_schemas_security_and_pons_metadata() {
    let document = fixture();
    let encoded = crate::emit(&document).unwrap();
    assert!(encoded.ends_with('\n'));
    let output: Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(output["openapi"], "3.1.1");
    assert_eq!(output["info"]["title"], "Example");
    assert_eq!(output["servers"][0]["url"], "https://api.example.test");
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["operationId"],
        "auth.login"
    );
    assert!(
        output["paths"]["/auth/login"]["post"]["security"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/LoginRequest"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["parameters"][0]["in"],
        "path"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["parameters"][1]["name"],
        "limit"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"]["text/event-stream"]
            ["schema"]["x-pons-events"][0]["event"],
        "user"
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["headers"]["X-Trace-ID"]["required"],
        true
    );
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["x-pons"]["stream"]["kind"],
        "sse"
    );
    assert!(
        output["paths"]["/events/{cursor}"]["get"]["security"][0]["Tenant"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["responses"]["4XX"]["content"]["application/json"]["schema"]
            ["properties"]["code"]["const"],
        "invalid"
    );
    assert_eq!(
        output["components"]["securitySchemes"]["Tenant"]["name"],
        "X-Tenant"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["id"]["type"],
        "string"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["id"]["x-pons"]["field"]["ident"][0],
        "id"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["x-pons"]["type"]["name"][0],
        "user"
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["nickname"]["anyOf"][1]["type"],
        "null"
    );
    let locked = output["components"]["schemas"]["LoginError"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|schema| schema["properties"]["code"]["const"] == "locked")
        .unwrap();
    assert!(locked["properties"]["retry_after"]["format"].is_null());
    let email = &output["components"]["schemas"]["LoginRequest"]["properties"]["email"];
    assert_eq!(email["x-pons-format"], json!("email"));
    assert!(email.get("format").is_none(), "{email:#}");
    assert!(
        output["x-pons"]["ext"]["test.example.source/v1"]["kept"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn preserves_operation_id_punctuation() {
    let mut document = fixture();
    document.operations[0].id = vec![
        vec!["under_score".into()],
        vec!["dot.name".into()],
        vec!["percent%name".into()],
    ];
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["operationId"],
        "under_score.dot.name.percent%name"
    );
}

#[test]
fn rejects_operation_id_collisions() {
    let mut document = fixture();
    document.operations[0].id = vec![vec!["same".into()], vec!["part".into()]];
    document.operations[1].id = vec![vec!["same.part".into()]];
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("duplicate OpenAPI operationId same.part")
    );
}

#[test]
fn rejects_mismatched_byte_stream_media_type_override() {
    let mut document = fixture();
    {
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = StreamKind::Bytes {
            content_type: "application/pdf".into(),
        };
        operation.responses[0].media =
            Some(pons_ir::MediaDescriptor::from_declared("application/json"));
    }
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("does not match stream kind media type")
    );

    document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .responses[0]
        .media = Some(pons_ir::MediaDescriptor::from_declared("application/pdf"));
    assert!(crate::emit(&document).is_ok());

    {
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = StreamKind::Bytes {
            content_type: "Application/PDF ; Version=1".into(),
        };
        operation.responses[0].media = Some(pons_ir::MediaDescriptor::from_declared(
            "application/pdf; version=1",
        ));
    }
    assert!(crate::emit(&document).is_ok());
}

#[test]
fn rejects_sse_and_ndjson_media_type_overrides() {
    let mut document = fixture();
    for (kind, expected) in [
        (StreamKind::Sse { events: vec![] }, "text/event-stream"),
        (
            StreamKind::Ndjson {
                item: TypeRef::Inline(InlineType::Bool),
            },
            "application/x-ndjson",
        ),
    ] {
        for (media_type, accepted) in [
            (expected.to_owned(), true),
            (format!("{expected}; charset=utf-8"), true),
            (format!("{expected} ; CharSet=UTF-8"), true),
            ("application/json".into(), false),
            ("application/octet-stream".into(), false),
        ] {
            let operation = document
                .operations
                .iter_mut()
                .find(|operation| operation.stream.is_some())
                .unwrap();
            operation.stream.as_mut().unwrap().kind = kind.clone();
            operation.responses[0].media =
                Some(pons_ir::MediaDescriptor::from_declared(media_type.clone()));
            assert_eq!(
                crate::emit(&document).is_ok(),
                accepted,
                "{media_type:?} on an {expected} stream"
            );
        }

        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = kind;
        operation.responses[0].media = None;
        let emitted: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert!(emitted.to_string().contains(expected));
    }
}

#[test]
fn emits_recursive_selected_enum_variants() {
    let mut raw: Value = serde_json::to_value(fixture()).unwrap();
    raw["types"]["Chain"] = json!({
        "kind": "enum",
        "name": ["chain"],
        "direction": "both",
        "repr": {"kind": "external"},
        "variants": [
            {
                "wire": "next",
                "ident": ["next"],
                "payload": {"kind": "newtype", "ty": {"ref": "Chain", "variant": "next"}}
            },
            {"wire": "leaf", "ident": ["leaf"], "payload": {"kind": "unit"}}
        ]
    });
    raw["types"]["User"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "wire": "chain",
            "ident": ["chain"],
            "ty": {"ref": "Chain", "variant": "next"},
            "presence": "required"
        }));
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let output = crate::emit(&document).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["openapi"], "3.1.1");
    assert!(
        value["components"]["schemas"]["Chain"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|schema| schema
                .to_string()
                .contains("#/components/schemas/Chain/oneOf/"))
    );
}

#[test]
fn emits_declared_error_response_headers() {
    let mut document = fixture();
    document.operations[0].errors[0]
        .headers
        .push(ResponseHeader {
            wire: "Retry-After".into(),
            ty: string(),
            presence: Presence::Required,
            ext: Default::default(),
        });
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let header =
        &output["paths"]["/auth/login"]["post"]["responses"]["4XX"]["headers"]["Retry-After"];
    assert_eq!(header["required"], true);
    assert_eq!(header["schema"]["type"], "string");
}

#[test]
fn emits_auth_scopes_only_for_oauth2_schemes() {
    let mut document = fixture();
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![OAuthFlow {
                kind: OAuthFlowKind::AuthorizationCode,
                authorization_url: Some("https://auth.test/authorize".into()),
                token_url: Some("https://auth.test/token".into()),
                scopes: BTreeMap::from([("events:read".into(), "Read events".into())]),
                ext: Default::default(),
            }],
        },
    );
    let operation = document
        .operations
        .iter_mut()
        .find(|operation| operation.id == vec![vec!["events"], vec!["watch"]])
        .unwrap();
    operation.auth.alternatives = vec![vec!["oauth".into()], vec!["tenant".into()]];
    operation.auth.scopes = BTreeMap::from([
        ("oauth".into(), vec!["events:read".into()]),
        ("tenant".into(), vec!["ignored".into()]),
    ]);
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let security = &output["paths"]["/events/{cursor}"]["get"]["security"];
    assert_eq!(security[0]["Oauth"][0], "events:read");
    assert_eq!(security[1]["Tenant"], json!([]));
}

#[test]
fn emits_object_openness_policies() {
    let mut document = fixture();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &user.definition
    else {
        unreachable!()
    };
    user.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveTyped { value: string() },
    };
    let login = document.types.get_mut("LoginRequest").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &login.definition
    else {
        unreachable!()
    };
    login.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveOpaque,
    };
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["components"]["schemas"]["User"]["additionalProperties"]["type"],
        "string"
    );
    assert_eq!(
        output["components"]["schemas"]["LoginRequest"]["additionalProperties"],
        json!(true)
    );
    assert_eq!(
        output["components"]["schemas"]["EventQuery"]["additionalProperties"],
        json!(false)
    );
}

#[test]
fn rejects_nullable_flatten_before_emitting_a_contract() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap();
    raw["types"]["LoginRequest"]["flattened"] = serde_json::json!([{
        "kind": "nullable",
        "inner": {"ref": "User"}
    }]);
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    for mode in [crate::EmitMode::Strict, crate::EmitMode::Lossy] {
        assert!(
            crate::emit_with_mode(&document, mode)
                .unwrap_err()
                .0
                .contains("nullable flattened")
        );
    }
}

#[test]
fn emits_optional_request_bodies() {
    let mut document = fixture();
    let login = document
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap();
    login.body.as_mut().unwrap().required = false;
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/auth/login"]["post"]["requestBody"]["required"],
        false
    );
}

#[test]
fn emits_success_status_variants() {
    let mut document = fixture();
    let login = document
        .operations
        .iter()
        .position(|operation| operation.id[0][0] == "auth")
        .unwrap();
    document.operations[login].responses[0].status = ResponseStatus::Class(2);
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert!(output["paths"]["/auth/login"]["post"]["responses"]["2XX"].is_object());
    document.operations[login].responses[0].status = ResponseStatus::Unknown;
    document.operations[login].errors[0].status = ErrorStatus::Exact(503);
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let responses = &output["paths"]["/auth/login"]["post"]["responses"];
    assert_eq!(responses["default"]["x-pons"]["status"], "unknown");
    assert!(responses.get("2XX").is_none());
    assert!(responses["503"].is_object());
    document.operations[login].responses[0].status = ResponseStatus::Default;
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let responses = &output["paths"]["/auth/login"]["post"]["responses"];
    assert_eq!(responses["default"]["x-pons"]["status"], "default");
    assert!(responses.get("2XX").is_none());
    assert!(responses["503"].is_object());
}

#[test]
fn escapes_path_literals_like_the_typescript_client() {
    let mut document = fixture();
    document.operations[0].path = vec![PathSegment::Literal {
        lit: "a b?c#dé%f".into(),
    }];
    document.operations[1].path = vec![PathSegment::Literal {
        lit: "a%20b".into(),
    }];
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let paths = output["paths"].as_object().unwrap();
    assert!(paths.contains_key("/a%20b%3Fc%23d%C3%A9%25f"), "{paths:?}");
    assert!(paths.contains_key("/a%2520b"), "{paths:?}");
}

#[test]
fn rejects_default_success_and_error_dispatch_collision() {
    let mut document = fixture();
    document.operations[0].responses[0].status = ResponseStatus::Default;
    document.operations[0].errors[0].status = ErrorStatus::Default;
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("collides with a success response")
    );
}

#[test]
fn rejects_opaque_query_objects() {
    let mut document = fixture();
    let query = document.types.get_mut("EventQuery").unwrap();
    let TypeKind::Struct {
        fields, flattened, ..
    } = &query.definition
    else {
        unreachable!()
    };
    query.definition = TypeKind::Struct {
        fields: fields.clone(),
        flattened: flattened.clone(),
        openness: ObjectOpenness::PreserveOpaque,
    };
    assert!(crate::emit(&document).is_err());
}

#[test]
fn emits_exact_custom_media_types() {
    let mut document = fixture();
    let login = document
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap();
    login.body.as_mut().unwrap().media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme+json",
    ));
    login.responses[0].media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme.session+json",
    ));
    login.errors[0].media = Some(pons_ir::MediaDescriptor::from_declared(
        "application/vnd.acme.error+json",
    ));
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let login = &output["paths"]["/auth/login"]["post"];
    assert!(login["requestBody"]["content"]["application/vnd.acme+json"].is_object());
    assert!(login["responses"]["200"]["content"]["application/vnd.acme.session+json"].is_object());
    assert!(login["responses"]["4XX"]["content"]["application/vnd.acme.error+json"].is_object());
}

#[test]
fn distinguishes_raw_byte_bodies_from_encoded_values() {
    let mut document = fixture();
    let user = document.types.get_mut("User").unwrap();
    let TypeKind::Struct { fields, .. } = &mut user.definition else {
        unreachable!()
    };
    fields.push(pons_ir::Field {
        wire: "avatar".into(),
        ident: vec!["avatar".into()],
        ty: TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Base64url,
        }),
        presence: pons_ir::Presence::Optional,
        default: Default::default(),
        access: None,
        doc: None,
        diagnostics: Vec::new(),
        ext: Default::default(),
    });
    let login = document
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap();
    login.body.as_mut().unwrap().content = ContentKind::Bytes;
    login.body.as_mut().unwrap().ty = TypeRef::Inline(InlineType::Bytes {
        encoding: ByteEncoding::Base64,
    });
    login.responses[0].content = ContentKind::Bytes;
    login.responses[0].body = Some(TypeRef::Inline(InlineType::Bytes {
        encoding: ByteEncoding::Hex,
    }));
    let events = document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    events.stream.as_mut().unwrap().kind = StreamKind::Bytes {
        content_type: "application/pdf".into(),
    };

    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let login = &output["paths"]["/auth/login"]["post"];
    assert_eq!(
        login["requestBody"]["content"]["application/octet-stream"]["schema"],
        json!({})
    );
    assert_eq!(login["requestBody"]["x-pons"]["ty"]["kind"], "bytes");
    assert_eq!(
        login["responses"]["200"]["content"]["application/octet-stream"]["schema"],
        json!({})
    );
    assert_eq!(login["responses"]["200"]["x-pons"]["body"]["kind"], "bytes");
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"]["application/pdf"]
            ["schema"],
        json!({"x-pons-stream":{"kind":"bytes","content_type":"application/pdf"}})
    );
    assert_eq!(
        output["components"]["schemas"]["User"]["properties"]["avatar"]["contentEncoding"],
        "base64url"
    );

    let mut wrapped = document.clone();
    let mut byte_alias = named_type("rawBytes");
    byte_alias.definition = TypeKind::Newtype {
        inner: TypeRef::Inline(InlineType::Bytes {
            encoding: ByteEncoding::Base64,
        }),
    };
    wrapped.types.insert("RawBytes".into(), byte_alias);
    wrapped
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap()
        .body
        .as_mut()
        .unwrap()
        .ty = TypeRef::Named(NamedRef {
        id: "RawBytes".into(),
        variant: None,
    });
    crate::emit(&wrapped).unwrap();

    let mut invalid = document.clone();
    invalid
        .operations
        .iter_mut()
        .find(|operation| operation.id[0][0] == "auth")
        .unwrap()
        .body
        .as_mut()
        .unwrap()
        .ty = string();
    assert!(
        crate::emit(&invalid)
            .unwrap_err()
            .0
            .contains("raw byte bodies must use a byte type")
    );

    let mut invalid = document;
    invalid
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .stream
        .as_mut()
        .unwrap()
        .kind = StreamKind::Bytes {
        content_type: String::new(),
    };
    assert!(
        crate::emit(&invalid)
            .unwrap_err()
            .0
            .contains("invalid byte stream content type")
    );
}

#[test]
fn unwraps_transparent_query_types_and_rejects_ambiguous_fields() {
    let mut document = fixture();
    let mut wrapper = named_type("eventQueryInput");
    wrapper.direction = Direction::Input;
    wrapper.definition = TypeKind::Newtype {
        inner: TypeRef::Named(NamedRef {
            id: "EventQuery".into(),
            variant: None,
        }),
    };
    document.types.insert("EventQueryInput".into(), wrapper);
    let events = document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    events.query = Some(TypeRef::Named(NamedRef {
        id: "EventQueryInput".into(),
        variant: None,
    }));

    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["parameters"][1]["name"],
        "limit"
    );

    let TypeKind::Struct { fields, .. } =
        &mut document.types.get_mut("EventQuery").unwrap().definition
    else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Array {
        item: Box::new(string()),
        unique: None,
    });
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("query fields must use scalar types")
    );
}

#[test]
fn rejects_auth_and_stream_contracts_openapi_cannot_describe() {
    let mut document = fixture();
    document.auth_schemes.insert(
        "tenant".into(),
        AuthScheme::Custom {
            description: "custom auth".into(),
            adapter: None,
            ext: Default::default(),
        },
    );
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("custom auth scheme")
    );

    let mut document = fixture();
    document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .responses
        .clear();
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("1 stream descriptions and 0 stream responses")
    );
}

#[test]
fn rejects_ambiguous_paths_headers_and_oauth_urls() {
    let mut document = fixture();
    document.operations[0].path[0] = PathSegment::Literal {
        lit: "auth/{login}".into(),
    };
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("template syntax")
    );

    let mut document = fixture();
    document.operations[0].path = vec![
        PathSegment::Literal {
            lit: "events".into(),
        },
        PathSegment::Param {
            param: "id".into(),
            ty: string(),
        },
    ];
    document.operations[1].path = vec![
        PathSegment::Literal {
            lit: "events".into(),
        },
        PathSegment::Param {
            param: "name".into(),
            ty: string(),
        },
    ];
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("OpenAPI route pattern /events/{} uses conflicting parameter names")
    );

    let mut document = fixture();
    document.operations[1].path = vec![PathSegment::Param {
        param: "bad/name".into(),
        ty: string(),
    }];
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("not a valid OpenAPI path parameter name")
    );

    let mut document = fixture();
    document.operations[1].headers[0].wire = "Accept".into();
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("request header Accept is owned by the request body or the security model")
    );

    let mut document = fixture();
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![OAuthFlow {
                kind: OAuthFlowKind::ClientCredentials,
                authorization_url: None,
                token_url: Some("http://identity.example.test/token".into()),
                scopes: Default::default(),
                ext: Default::default(),
            }],
        },
    );
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("OAuth2 token URL for oauth must be an absolute HTTPS URL")
    );

    let mut document = fixture();
    document.auth_schemes.insert(
        "oauth".into(),
        AuthScheme::OAuth2 {
            flows: vec![OAuthFlow {
                kind: OAuthFlowKind::ClientCredentials,
                authorization_url: None,
                token_url: Some("https://?query".into()),
                scopes: Default::default(),
                ext: Default::default(),
            }],
        },
    );
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("OAuth2 token URL for oauth must be an absolute HTTPS URL")
    );
}

#[test]
fn canonicalizes_equivalent_documents_before_emitting() {
    let mut document = fixture();
    let operation = document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    operation.auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];
    let expected = crate::emit(&document).unwrap();
    let mut shuffled = document.clone();
    shuffled.operations.reverse();
    shuffled
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap()
        .auth
        .alternatives
        .reverse();

    assert_eq!(crate::emit(&shuffled).unwrap(), expected);
}
