use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../../fixtures/login.json")).unwrap()
}

fn level(old: &Document, changed: serde_json::Value, path: &str) -> ChangeLevel {
    let new: Document = serde_json::from_value(changed).unwrap();
    old.diff(&new)
        .changes
        .into_iter()
        .find(|change| change.path == path)
        .unwrap()
        .level
}

fn change_at(old: &Document, new: &Document, path: &str) -> Change {
    old.diff(new)
        .changes
        .into_iter()
        .find(|change| change.path == path)
        .unwrap_or_else(|| panic!("no change reported at {path}"))
}

#[test]
fn equivalent_orderings_have_no_diff() {
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    let event = old
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    event.auth.alternatives = vec![vec!["session".into()], vec!["tenant".into()]];
    let mut second_header = event.responses[0].headers[0].clone();
    second_header.wire = "X-Second".into();
    event.responses[0].headers.push(second_header);
    let flow = |kind| crate::OAuthFlow {
        kind,
        authorization_url: None,
        token_url: Some("https://identity.example.test/token".into()),
        scopes: Default::default(),
        ext: Default::default(),
    };
    old.auth_schemes.insert(
        "oauth".into(),
        crate::AuthScheme::OAuth2 {
            flows: vec![
                flow(crate::OAuthFlowKind::Password),
                flow(crate::OAuthFlowKind::ClientCredentials),
            ],
        },
    );

    let mut new = old.clone();
    new.operations.reverse();
    let event = &mut new.operations[0];
    event.auth.alternatives.reverse();
    event.responses[0].headers.reverse();
    let crate::AuthScheme::OAuth2 { flows } = new.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows.reverse();
    old.validate().unwrap();
    new.validate().unwrap();

    assert_eq!(old.diff(&new), Diff::default());
}

#[test]
fn flattened_reordering_has_no_diff() {
    let member = |id: &str| {
        serde_json::json!({
            "kind": "struct",
            "name": [id],
            "direction": "output",
            "openness": "reject",
            "fields": [{
                "wire": id,
                "ident": [id],
                "ty": {"kind": "string"},
                "presence": "optional"
            }]
        })
    };
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    for id in ["Meta", "Trace"] {
        old.types
            .insert(id.into(), serde_json::from_value(member(id)).unwrap());
    }
    let TypeKind::Struct { flattened, .. } = &mut old.types.get_mut("User").unwrap().definition
    else {
        unreachable!()
    };
    *flattened = vec![
        TypeRef::Named(crate::NamedRef {
            id: "Meta".into(),
            variant: None,
        }),
        TypeRef::Named(crate::NamedRef {
            id: "Trace".into(),
            variant: None,
        }),
    ];

    let mut new = old.clone();
    let TypeKind::Struct { flattened, .. } = &mut new.types.get_mut("User").unwrap().definition
    else {
        unreachable!()
    };
    flattened.reverse();
    old.validate().unwrap();
    new.validate().unwrap();

    assert_eq!(old.diff(&new), Diff::default());
}

#[test]
fn worst_reports_the_highest_level() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    assert_eq!(old.diff(&old).worst(), None);

    let mut warned = old.clone();
    warned.operations[0]
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    let diff = old.diff(&warned);
    assert_eq!(diff.worst(), Some(ChangeLevel::Warning));
    assert!(!diff.has_breaking_changes());

    let mut broken = warned;
    broken.operations[0].semantics.idempotent = true;
    let diff = old.diff(&broken);
    assert_eq!(diff.worst(), Some(ChangeLevel::Breaking));
    assert!(diff.has_breaking_changes());
}

#[test]
fn field_presence_is_graded_against_the_new_direction() {
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    old.types.insert(
        "Profile".into(),
        serde_json::from_value(serde_json::json!({
            "kind": "struct",
            "name": ["profile"],
            "direction": "output",
            "openness": "reject",
            "fields": [{
                "wire": "handle",
                "ident": ["handle"],
                "ty": {"kind": "string"},
                "presence": "optional"
            }]
        }))
        .unwrap(),
    );
    let mut new: Document = serde_json::from_value(serde_json::to_value(&old).unwrap()).unwrap();
    let definition = new.types.get_mut("Profile").unwrap();
    definition.direction = Direction::Input;
    let TypeKind::Struct { fields, .. } = &mut definition.definition else {
        unreachable!()
    };
    fields[0].presence = Presence::Required;
    old.validate().unwrap();
    new.validate().unwrap();

    let change = change_at(&old, &new, "types.Profile.fields.handle");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "field presence changed");
    assert_eq!(old.diff(&new).worst(), Some(ChangeLevel::Breaking));
    assert_eq!(
        change_at(&old, &new, "types.Profile").detail,
        "type direction changed"
    );
}

#[test]
fn a_required_read_only_input_field_is_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["EventQuery"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "wire": "since",
            "ident": ["since"],
            "ty": {"kind": "string"},
            "presence": "required",
            "access": "read_only"
        }));
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(new.validate().is_err());

    let change = change_at(&old, &new, "types.EventQuery.fields.since");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "field added");
}

fn read_only_field(presence: &str) -> serde_json::Value {
    serde_json::json!({
        "wire": "since",
        "ident": ["since"],
        "ty": {"kind": "string"},
        "presence": presence,
        "access": "read_only"
    })
}

#[test]
fn a_required_read_only_field_on_an_output_carrier_is_breaking() {
    let mut old = fixture();
    old["types"]["EventQuery"]["direction"] = serde_json::json!("output");
    let old: Document = serde_json::from_value(old).unwrap();
    let mut changed = serde_json::to_value(&old).unwrap();
    changed["types"]["EventQuery"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(read_only_field("required"));
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(new.validate().is_err());

    let diff = old.diff(&new);
    let change = change_at(&old, &new, "types.EventQuery.fields.since");
    assert_eq!(change.level, ChangeLevel::Breaking, "{:#?}", diff.changes);
    assert_eq!(change.detail, "field added");
    assert_eq!(
        change_at(&old, &new, "document").level,
        ChangeLevel::Warning
    );
}

#[test]
fn a_required_read_only_field_behind_a_flattened_carrier_is_breaking() {
    let mut old = fixture();
    old["types"]["EventQuery"]["flattened"] = serde_json::json!([{"ref": "EventFilter"}]);
    old["types"]["EventFilter"] = serde_json::json!({
        "kind": "struct",
        "name": ["event", "filter"],
        "direction": "input",
        "openness": "reject",
        "fields": [{
            "wire": "kind",
            "ident": ["kind"],
            "ty": {"kind": "string"},
            "presence": "optional"
        }]
    });
    let old: Document = serde_json::from_value(old).unwrap();
    old.validate().unwrap();

    let mut changed = serde_json::to_value(&old).unwrap();
    changed["types"]["EventFilter"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(read_only_field("required"));
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(new.validate().is_err());

    let diff = old.diff(&new);
    let change = change_at(&old, &new, "types.EventFilter.fields.since");
    assert_eq!(change.level, ChangeLevel::Breaking, "{:#?}", diff.changes);
    assert_eq!(change.detail, "field added");
    assert_eq!(
        change_at(&old, &new, "document").level,
        ChangeLevel::Breaking,
        "the field and the document it invalidates must not disagree"
    );
}

#[test]
fn a_required_read_only_field_behind_a_newtype_carrier_is_breaking() {
    let mut old = fixture();
    old["types"]["EventWindow"] = serde_json::json!({
        "kind": "newtype",
        "name": ["event", "window"],
        "direction": "input",
        "inner": {"ref": "EventQuery"}
    });
    old["operations"][1]["query"] = serde_json::json!({"ref": "EventWindow"});
    let old: Document = serde_json::from_value(old).unwrap();
    old.validate().unwrap();

    let mut changed = serde_json::to_value(&old).unwrap();
    changed["types"]["EventQuery"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(read_only_field("required"));
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(new.validate().is_err());

    let diff = old.diff(&new);
    let change = change_at(&old, &new, "types.EventQuery.fields.since");
    assert_eq!(change.level, ChangeLevel::Breaking, "{:#?}", diff.changes);
    assert_eq!(change.detail, "field added");
    assert_eq!(
        change_at(&old, &new, "document").level,
        ChangeLevel::Breaking
    );
}

#[test]
fn a_carrier_grades_presence_and_default_changes_as_input() {
    let mut old = fixture();
    old["types"]["EventQuery"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(read_only_field("optional"));
    let old: Document = serde_json::from_value(old).unwrap();
    old.validate().unwrap();

    let mut required = serde_json::to_value(&old).unwrap();
    required["types"]["EventQuery"]["fields"][1]["presence"] = serde_json::json!("required");
    let new: Document = serde_json::from_value(required).unwrap();
    assert!(new.validate().is_err());
    let diff = old.diff(&new);
    assert_eq!(
        change_at(&old, &new, "types.EventQuery.fields.since").level,
        ChangeLevel::Breaking,
        "{:#?}",
        diff.changes
    );
    assert_eq!(
        change_at(&old, &new, "document").level,
        ChangeLevel::Breaking
    );

    let mut bidirectional = fixture();
    bidirectional["types"]["User"]["direction"] = serde_json::json!("both");
    bidirectional["types"]["User"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(read_only_field("optional"));
    let sent: Document = serde_json::from_value(bidirectional).unwrap();
    sent.validate().unwrap();

    let mut defaulted = serde_json::to_value(&sent).unwrap();
    defaulted["types"]["User"]["fields"][2]["default"] = serde_json::json!("yesterday");
    let new: Document = serde_json::from_value(defaulted).unwrap();
    new.validate().unwrap();
    let change = change_at(&sent, &new, "types.User.fields.since");
    assert_eq!(change.level, ChangeLevel::Breaking, "{change:#?}");
    assert_eq!(change.detail, "field default changed");
}

#[test]
fn an_invalid_side_is_reported_once() {
    let old: Document = serde_json::from_value(fixture()).unwrap();

    let mut carrier = fixture();
    carrier["operations"][1]["headers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "wire": "X-Tenant",
            "ident": ["tenant"],
            "ty": {"kind": "string"},
            "presence": "optional"
        }));
    let conflicting: Document = serde_json::from_value(carrier).unwrap();
    assert!(conflicting.validate().is_err());

    let diff = old.diff(&conflicting);
    let reported = diff
        .changes
        .iter()
        .filter(|change| change.path == "document")
        .collect::<Vec<_>>();
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].level, ChangeLevel::Breaking);
    assert!(
        reported[0]
            .detail
            .starts_with("the new document is invalid: ")
    );
    assert!(reported[0].detail.contains("apiKey carrier name"));
    assert_eq!(
        change_at(
            &old,
            &conflicting,
            "operations.events.watch.headers.X-Tenant"
        )
        .level,
        ChangeLevel::NonBreaking
    );
    assert_eq!(diff.worst(), Some(ChangeLevel::Breaking));

    let mut contradicted = fixture();
    contradicted["types"]["User"]["fields"][1]["default"] = serde_json::json!(7);
    let invalid: Document = serde_json::from_value(contradicted).unwrap();
    assert!(invalid.validate().is_err());
    assert_eq!(
        change_at(&old, &invalid, "types.User.fields.nickname").level,
        ChangeLevel::NonBreaking
    );
    let change = change_at(&old, &invalid, "document");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert!(change.detail.contains("contradicts its type"));

    let mut diagnosed = fixture();
    diagnosed["types"]["User"]["diagnostics"] = serde_json::json!([{
        "severity": "error",
        "code": "type.x",
        "message": "unresolved"
    }]);
    let broken: Document = serde_json::from_value(diagnosed).unwrap();
    assert!(broken.validate().is_err());

    let change = change_at(&broken, &old, "document");
    assert_eq!(change.level, ChangeLevel::Info);
    assert!(
        change
            .detail
            .starts_with("the previous document was invalid: ")
    );
    assert!(change.detail.contains("error diagnostic type.x"));
}

#[test]
fn a_required_read_only_field_refuses_the_document_in_every_carrier() {
    let field = serde_json::json!({
        "wire": "since",
        "ident": ["since"],
        "ty": {"kind": "string"},
        "presence": "required",
        "access": "read_only"
    });
    let refusal = "the client drops the field on encode and every server built from the document waits for a key the client cannot send";

    let mut bidirectional = fixture();
    bidirectional["types"]["User"]["direction"] = serde_json::json!("both");
    let old: Document = serde_json::from_value(bidirectional).unwrap();
    let mut changed = serde_json::to_value(&old).unwrap();
    changed["types"]["User"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(field.clone());
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(rejections(&new).iter().any(|error| error.contains(refusal)));
    assert_eq!(
        change_at(&old, &new, "types.User.fields.since").level,
        ChangeLevel::Breaking
    );
    assert_eq!(
        change_at(&old, &new, "document").level,
        ChangeLevel::Breaking
    );

    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["LoginRequest"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(field);
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(rejections(&new).iter().any(|error| error.contains(refusal)));
    assert_eq!(
        change_at(&old, &new, "document").level,
        ChangeLevel::Breaking
    );
}

fn rejections(document: &Document) -> Vec<String> {
    document
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn an_added_error_diagnostic_is_reported_next_to_the_validity_change() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["diagnostics"] = serde_json::json!([
        {"severity": "error", "code": "type.x", "message": "first"},
        {"severity": "error", "code": "type.x", "message": "second"},
    ]);
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(new.validate().is_err());

    let diff = old.diff(&new);
    let validity = diff
        .changes
        .iter()
        .find(|change| change.path == "document")
        .unwrap();
    assert_eq!(validity.level, ChangeLevel::Breaking);
    assert!(validity.detail.starts_with("the new document is invalid: "));
    assert!(validity.detail.contains("error diagnostic type.x: first"));
    assert!(validity.detail.contains("error diagnostic type.x: second"));
    let added = diff
        .changes
        .iter()
        .filter(|change| change.detail == "error diagnostic added: type.x")
        .collect::<Vec<_>>();
    assert_eq!(added.len(), 2, "{:#?}", diff.changes);
    assert!(
        added
            .iter()
            .all(|change| change.level == ChangeLevel::Breaking)
    );
    assert!(!diff.validity.new_is_valid());
}

#[test]
fn two_invalid_documents_are_reported_by_both_sides() {
    let mut contradicted = fixture();
    contradicted["types"]["User"]["fields"][1]["default"] = serde_json::json!(7);
    let old: Document = serde_json::from_value(contradicted).unwrap();
    let mut also_broken = serde_json::to_value(&old).unwrap();
    also_broken["info"]["title"] = "Renamed".into();
    also_broken["auth_schemes"]["broken"] = serde_json::json!({
        "kind": "api_key",
        "in": "query",
        "name": ""
    });
    let new: Document = serde_json::from_value(also_broken).unwrap();
    assert!(old.validate().is_err());
    assert!(new.validate().is_err());

    let diff = old.diff(&new);
    let change = change_at(&old, &new, "document");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert!(change.detail.starts_with("both documents are invalid: "));
    assert!(change.detail.contains("contradicts its type"));
    assert!(change.detail.contains("carrier names cannot be empty"));
    assert_eq!(diff.worst(), Some(ChangeLevel::Warning));

    assert!(!diff.validity.new_is_valid());
    assert!(!diff.validity.previous_is_valid());
    assert!(
        diff.validity
            .previous
            .iter()
            .any(|error| error.to_string().contains("contradicts its type")),
        "{:?}",
        diff.validity.previous
    );
    assert!(
        diff.validity
            .new
            .iter()
            .any(|error| error.to_string().contains("carrier names cannot be empty")),
        "{:?}",
        diff.validity.new
    );

    assert_eq!(
        diff.validity.previous.len(),
        1,
        "{:?}",
        diff.validity.previous
    );
    assert_eq!(diff.validity.new.len(), 2, "{:?}", diff.validity.new);

    let valid: Document = serde_json::from_value(fixture()).unwrap();
    let unchanged = valid.diff(&valid);
    assert!(unchanged.validity.new_is_valid());
    assert!(unchanged.validity.previous_is_valid());
    assert_eq!(unchanged, Diff::default());
}

fn set_diagnostics(diagnostics: &mut Vec<Diagnostic>, severity: Option<DiagnosticSeverity>) {
    diagnostics.clear();
    diagnostics.extend(severity.map(|severity| Diagnostic {
        severity,
        code: "type.x".into(),
        message: "unresolved".into(),
        origin: None,
        ext: BTreeMap::new(),
    }));
}

type DiagnosticSite = (&'static str, fn(&mut Document, Option<DiagnosticSeverity>));

#[test]
fn diagnostic_changes_are_reported_at_their_own_site() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let carriers: [DiagnosticSite; 3] = [
        ("types.User", |document, severity| {
            set_diagnostics(
                &mut document.types.get_mut("User").unwrap().diagnostics,
                severity,
            )
        }),
        ("operations.auth.login", |document, severity| {
            set_diagnostics(&mut document.operations[0].diagnostics, severity)
        }),
        ("document.diagnostics", |document, severity| {
            set_diagnostics(&mut document.diagnostics, severity)
        }),
    ];

    for (path, site) in carriers {
        let mut warned = old.clone();
        site(&mut warned, Some(DiagnosticSeverity::Warning));
        let mut escalated = warned.clone();
        site(&mut escalated, Some(DiagnosticSeverity::Error));

        let change = change_at(&warned, &escalated, path);
        assert_eq!(
            (change.level, change.detail.as_str()),
            (
                ChangeLevel::Breaking,
                "diagnostic severity escalated: type.x"
            ),
            "{path}"
        );

        let change = change_at(&escalated, &warned, path);
        assert_eq!(
            (change.level, change.detail.as_str()),
            (
                ChangeLevel::NonBreaking,
                "diagnostic severity lowered: type.x"
            ),
            "{path}"
        );

        let mut repaired = escalated.clone();
        site(&mut repaired, None);
        let change = change_at(&escalated, &repaired, path);
        assert_eq!(
            (change.level, change.detail.as_str()),
            (ChangeLevel::NonBreaking, "error diagnostic removed: type.x"),
            "{path}"
        );
        assert!(!escalated.diff(&repaired).has_breaking_changes(), "{path}");
    }
}

#[test]
fn a_reworded_diagnostic_is_a_change_and_not_a_removal() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut errored = old.clone();
    errored
        .types
        .get_mut("User")
        .unwrap()
        .diagnostics
        .push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: "type.x".into(),
            message: "the field is unresolved".into(),
            origin: None,
            ext: BTreeMap::new(),
        });
    assert!(errored.validate().is_err());

    let mut reworded = errored.clone();
    reworded.types.get_mut("User").unwrap().diagnostics[0].message =
        "the field cannot be resolved".into();
    let diff = errored.diff(&reworded);
    assert!(errored.validate().is_err(), "the error is still there");
    let reported = diff
        .changes
        .iter()
        .filter(|change| change.path != "document")
        .collect::<Vec<_>>();
    assert_eq!(reported.len(), 1, "{:#?}", diff.changes);
    assert_eq!(reported[0].level, ChangeLevel::NonBreaking);
    assert_eq!(reported[0].detail, "diagnostic message changed: type.x");
    assert!(
        !diff
            .changes
            .iter()
            .any(|change| change.detail.contains("removed")),
        "a reword is not a removal: {:#?}",
        diff.changes
    );
    assert!(errored.validate().is_err(), "the error is still there");
    assert!(reworded.validate().is_err(), "the error is still there");
    assert!(!errored.diff(&reworded).has_breaking_changes());
}

#[test]
fn diagnostic_severity_transitions_key_on_the_transition() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let warned = |severity| {
        let mut document = old.clone();
        document
            .types
            .get_mut("User")
            .unwrap()
            .diagnostics
            .push(Diagnostic {
                severity,
                code: "type.x".into(),
                message: "unresolved".into(),
                origin: None,
                ext: BTreeMap::new(),
            });
        document
    };
    let info = warned(DiagnosticSeverity::Info);
    let warning = warned(DiagnosticSeverity::Warning);

    let change = change_at(&info, &warning, "types.User");
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "diagnostic severity escalated: type.x");

    let change = change_at(&warning, &info, "types.User");
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "diagnostic severity lowered: type.x");

    let change = change_at(&info, &warned(DiagnosticSeverity::Error), "types.User");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "diagnostic severity escalated: type.x");
    assert!(
        info.diff(&warned(DiagnosticSeverity::Error))
            .has_breaking_changes()
    );
}

#[test]
fn type_field_and_variant_documentation_is_compared() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut deprecated = fixture();
    deprecated["types"]["User"]["doc"] = serde_json::json!({"deprecated": true});
    deprecated["types"]["User"]["fields"][0]["doc"] = serde_json::json!({"deprecated": true});
    deprecated["types"]["LoginError"]["variants"][0]["doc"] =
        serde_json::json!({"deprecated": true});
    let new: Document = serde_json::from_value(deprecated).unwrap();
    new.validate().unwrap();

    let diff = old.diff(&new);
    assert_eq!(diff.worst(), Some(ChangeLevel::Warning));
    for (path, label) in [
        ("types.User", "type"),
        ("types.User.fields.id", "field"),
        ("types.LoginError.variants.invalid", "enum variant"),
    ] {
        let change = diff
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no change reported at {path}"));
        assert_eq!(change.level, ChangeLevel::Warning);
        assert_eq!(change.detail, format!("{label} deprecation changed"));
    }

    let mut summarized = fixture();
    summarized["types"]["User"]["doc"] = serde_json::json!({"summary": "an account"});
    let new: Document = serde_json::from_value(summarized).unwrap();
    let change = change_at(&old, &new, "types.User");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "type documentation changed");
    assert!(!old.diff(&new).has_breaking_changes());
}

#[test]
fn added_error_diagnostics_are_reported() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["fields"][0]["diagnostics"] = serde_json::json!([{
        "severity": "error",
        "code": "field.x",
        "message": "unresolved"
    }]);
    changed["types"]["LoginError"]["variants"][0]["diagnostics"] = serde_json::json!([{
        "severity": "error",
        "code": "variant.x",
        "message": "unresolved"
    }]);
    let new: Document = serde_json::from_value(changed).unwrap();
    assert!(new.validate().is_err());

    let diff = old.diff(&new);
    let change = diff
        .changes
        .iter()
        .find(|change| change.path == "document")
        .unwrap();
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert!(
        change
            .detail
            .contains("error diagnostic field.x: unresolved")
    );
    assert!(
        change
            .detail
            .contains("error diagnostic variant.x: unresolved")
    );
    for (path, code) in [
        ("types.User.fields.id", "field.x"),
        ("types.LoginError.variants.invalid", "variant.x"),
    ] {
        let change = diff
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no change reported at {path}"));
        assert_eq!(change.level, ChangeLevel::Breaking, "{path}");
        assert_eq!(
            change.detail,
            format!("error diagnostic added: {code}"),
            "{path}"
        );
    }
}

#[test]
fn request_body_auth_and_provision_metadata_are_warnings() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut annotated = old.clone();
    annotated.operations[0]
        .body
        .as_mut()
        .unwrap()
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    annotated.operations[0]
        .auth
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    annotated.operations[0]
        .provides
        .as_mut()
        .unwrap()
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));

    let diff = old.diff(&annotated);
    assert_eq!(diff.worst(), Some(ChangeLevel::Warning));
    assert!(!diff.has_breaking_changes());
    for (path, detail) in [
        (
            "operations.auth.login.body",
            "request body metadata changed",
        ),
        (
            "operations.auth.login.auth",
            "authentication requirement metadata changed",
        ),
        (
            "operations.auth.login.provides",
            "credential provision metadata changed",
        ),
    ] {
        let change = diff
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no change reported at {path}"));
        assert_eq!(change.level, ChangeLevel::Warning);
        assert_eq!(change.detail, detail);
    }

    let mut retyped = annotated;
    retyped.operations[0].body.as_mut().unwrap().media_type =
        Some("application/vnd.acme+json".into());
    let diff = old.diff(&retyped);
    let body = diff
        .changes
        .iter()
        .filter(|change| change.path == "operations.auth.login.body")
        .collect::<Vec<_>>();
    assert!(
        body.iter()
            .any(|change| change.level == ChangeLevel::Breaking
                && change.detail == "request body changed")
    );
    assert!(
        body.iter()
            .any(|change| change.level == ChangeLevel::Warning
                && change.detail == "request body metadata changed")
    );
}

#[test]
fn stream_events_are_classified_per_event() {
    let old: Document = serde_json::from_value(fixture()).unwrap();

    let mut added = fixture();
    added["operations"][1]["stream"]["events"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"event": "audit", "data": {"ref": "User"}}));
    let new: Document = serde_json::from_value(added).unwrap();
    let change = change_at(&old, &new, "operations.events.watch.stream.events.audit");
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "stream event added");

    let mut renamed = fixture();
    renamed["operations"][1]["stream"]["events"][0]["event"] = serde_json::json!("person");
    let new: Document = serde_json::from_value(renamed).unwrap();
    let removed = change_at(&old, &new, "operations.events.watch.stream.events.user");
    assert_eq!(removed.level, ChangeLevel::Breaking);
    assert_eq!(removed.detail, "stream event removed");
    assert_eq!(
        change_at(&old, &new, "operations.events.watch.stream.events.person").detail,
        "stream event added"
    );

    let mut retyped = fixture();
    retyped["operations"][1]["stream"]["events"][0]["data"] = serde_json::json!({"kind": "string"});
    let new: Document = serde_json::from_value(retyped).unwrap();
    let change = change_at(&old, &new, "operations.events.watch.stream.events.user");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "stream event data type changed");
}

#[test]
fn non_event_stream_kinds_keep_their_own_paths() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let reskinned = |kind: crate::StreamKind| {
        let mut document = old.clone();
        document.operations[1].stream.as_mut().unwrap().kind = kind;
        document.validate().unwrap();
        document
    };
    let kind_changed = reskinned(crate::StreamKind::Ndjson {
        item: TypeRef::Inline(InlineType::Bool),
    });
    let change = change_at(&old, &kind_changed, "operations.events.watch.stream");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "stream kind changed");

    for (from, to, path, detail) in [
        (
            crate::StreamKind::Ndjson {
                item: TypeRef::Inline(InlineType::Bool),
            },
            crate::StreamKind::Ndjson {
                item: TypeRef::Inline(InlineType::Null),
            },
            "operations.events.watch.stream.item",
            "stream item type changed",
        ),
        (
            crate::StreamKind::Bytes {
                content_type: "application/octet-stream".into(),
            },
            crate::StreamKind::Bytes {
                content_type: "application/x-ndjson".into(),
            },
            "operations.events.watch.stream.content_type",
            "stream content type changed",
        ),
        (
            crate::StreamKind::Websocket {
                client: TypeRef::Inline(InlineType::Bool),
                server: TypeRef::Inline(InlineType::Bool),
            },
            crate::StreamKind::Websocket {
                client: TypeRef::Inline(InlineType::Null),
                server: TypeRef::Inline(InlineType::Bool),
            },
            "operations.events.watch.stream.client",
            "stream client message type changed",
        ),
    ] {
        let retyped = reskinned(to);
        let change = change_at(&reskinned(from), &retyped, path);
        assert_eq!(
            (change.level, change.detail.as_str()),
            (ChangeLevel::Breaking, detail)
        );
    }
}

#[test]
fn an_unchanged_stream_of_every_kind_has_no_diff() {
    let sse: Document = serde_json::from_value(fixture()).unwrap();
    let reskinned = |kind: crate::StreamKind| {
        let mut document = sse.clone();
        document.operations[1].stream.as_mut().unwrap().kind = kind;
        document.validate().unwrap();
        document
    };

    for document in [
        sse.clone(),
        reskinned(crate::StreamKind::Ndjson {
            item: TypeRef::Named(crate::NamedRef {
                id: "User".into(),
                variant: None,
            }),
        }),
        reskinned(crate::StreamKind::Bytes {
            content_type: "application/octet-stream".into(),
        }),
        reskinned(crate::StreamKind::Websocket {
            client: TypeRef::Inline(InlineType::Bool),
            server: TypeRef::Named(crate::NamedRef {
                id: "User".into(),
                variant: None,
            }),
        }),
    ] {
        assert_eq!(document.diff(&document), Diff::default());
    }
}

#[test]
fn error_change_paths_are_injective() {
    let old: Document = serde_json::from_value(fixture()).unwrap();

    let mut statuses = old.clone();
    statuses.operations[0].errors[0].doc = Some(doc(Some("Login failed"), None));
    let mut unavailable = statuses.operations[0].errors[0].clone();
    unavailable.status = crate::ErrorStatus::Class(5);
    statuses.operations[0].errors.push(unavailable);
    statuses.validate().unwrap();

    let mut paths = old
        .diff(&statuses)
        .changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            "operations.auth.login.errors.5xx.login_error".to_owned(),
            "operations.auth.login.errors.login_error".to_owned(),
        ]
    );

    let mut names = old.clone();
    let mut parts = names.operations[0].errors[0].clone();
    parts.name = vec!["a".into(), "b".into()];
    let mut joined = parts.clone();
    joined.status = crate::ErrorStatus::Class(5);
    joined.name = vec!["a_b".into()];
    names.operations[0].errors = vec![parts, joined];
    names.validate().unwrap();

    let mut paths = old
        .diff(&names)
        .changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            "operations.auth.login.errors.5xx.a%5Fb".to_owned(),
            "operations.auth.login.errors.a_b".to_owned(),
            "operations.auth.login.errors.login_error".to_owned(),
        ]
    );
}

#[test]
fn change_paths_stay_unique_across_the_lists_that_carry_them() {
    let record = |id: &str, wires: &[&str]| {
        serde_json::from_value(serde_json::json!({
            "kind": "struct",
            "name": [id],
            "direction": "output",
            "openness": "reject",
            "fields": wires.iter().map(|wire| serde_json::json!({
                "wire": wire,
                "ident": [wire],
                "ty": {"kind": "string"},
                "presence": "optional"
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    };

    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    old.types
        .insert("A".into(), record("A", &["x", "B.fields.c"]));
    old.types
        .insert("A.fields.B".into(), record("A_fields_B", &["x", "c"]));
    old.operations[0].errors[0].name = vec!["x".into()];
    let mut nested = old.operations[0].clone();
    nested.id = vec![
        vec!["auth".into()],
        vec!["login".into()],
        vec!["errors".into()],
        vec!["x".into()],
    ];
    nested.path = vec![
        crate::PathSegment::Literal {
            lit: "errors".into(),
        },
        crate::PathSegment::Param {
            param: "x".into(),
            ty: TypeRef::Inline(InlineType::Bool),
        },
    ];
    old.operations.push(nested);
    for id in ["a", "a.flows"] {
        old.auth_schemes.insert(
            id.into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: crate::OAuthFlowKind::Password,
                    authorization_url: None,
                    token_url: Some("https://identity.example.test/token".into()),
                    scopes: BTreeMap::new(),
                    ext: BTreeMap::new(),
                }],
            },
        );
    }
    old.validate().unwrap();

    let mut new = old.clone();
    for (id, wire) in [("A", "B.fields.c"), ("A.fields.B", "c")] {
        let TypeKind::Struct { fields, .. } = &mut new.types.get_mut(id).unwrap().definition else {
            unreachable!()
        };
        fields.retain(|field| field.wire != wire);
    }
    new.operations[0].errors.clear();
    new.operations.pop();
    for id in ["a", "a.flows"] {
        let AuthScheme::OAuth2 { flows } = new.auth_schemes.get_mut(id).unwrap() else {
            unreachable!()
        };
        flows[0].token_url = Some("https://identity.example.test/v2/token".into());
    }
    new.validate().unwrap();

    let changes = old.diff(&new).changes;
    let mut paths = changes
        .iter()
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();
    let count = paths.len();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), count, "{changes:#?}");
    for (path, detail) in [
        ("types.A.fields.B.fields.c", "field removed"),
        ("types.A%2Efields%2EB.fields.c", "field removed"),
        ("operations.auth.login.errors.x", "operation removed"),
        ("operations.auth.login.errors.4xx.x", "error removed"),
        ("auth_schemes.a.flows.Password", "OAuth2 token URL changed"),
        (
            "auth_schemes.a.flows.flows.Password",
            "OAuth2 token URL changed",
        ),
    ] {
        assert!(
            changes
                .iter()
                .any(|change| change.path == path && change.detail == detail),
            "no {detail:?} reported at {path} in {changes:#?}"
        );
    }
}

#[test]
fn a_key_with_a_dot_prints_readable_and_escapes_only_on_a_collision() {
    let record = |id: &str, wires: &[&str]| {
        serde_json::from_value(serde_json::json!({
            "kind": "struct",
            "name": [id],
            "direction": "output",
            "openness": "reject",
            "fields": wires.iter().map(|wire| serde_json::json!({
                "wire": wire,
                "ident": [wire],
                "ty": {"kind": "string"},
                "presence": "optional"
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    };
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    old.types
        .insert("A".into(), record("A", &["%b.fields.c", "f_o.fields.c"]));
    old.types
        .insert("A.fields.%b".into(), record("A_fields_percent_b", &["c"]));
    old.types
        .insert("A.fields.f_o".into(), record("A_fields_f_o", &["c"]));

    let mut new = old.clone();
    new.types.remove("A.fields.f_o");
    old.validate().unwrap();
    new.validate().unwrap();
    assert_eq!(
        change_at(&old, &new, "types.A.fields.f_o").detail,
        "type removed"
    );

    let mut new = old.clone();
    for (id, wire) in [
        ("A", "%b.fields.c"),
        ("A", "f_o.fields.c"),
        ("A.fields.%b", "c"),
        ("A.fields.f_o", "c"),
    ] {
        let TypeKind::Struct { fields, .. } = &mut new.types.get_mut(id).unwrap().definition else {
            unreachable!()
        };
        fields.retain(|field| field.wire != wire);
    }
    old.validate().unwrap();
    new.validate().unwrap();
    let removed = old
        .diff(&new)
        .changes
        .into_iter()
        .filter(|change| change.detail == "field removed")
        .map(|change| change.path)
        .collect::<Vec<_>>();
    assert_eq!(
        removed,
        [
            "types.A.fields.%b.fields.c",
            "types.A.fields.f_o.fields.c",
            "types.A%2Efields%2E%25b.fields.c",
            "types.A%2Efields%2Ef%5Fo.fields.c",
        ]
    );
}

#[test]
fn classifies_added_fields_and_output_enum_variants() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["LoginRequest"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "wire": "remember",
            "ident": ["remember"],
            "ty": {"kind": "bool"},
            "presence": "optional"
        }));
    assert_eq!(
        level(&old, changed, "types.LoginRequest.fields.remember"),
        ChangeLevel::NonBreaking
    );

    let mut changed = fixture();
    changed["types"]["LoginError"]["variants"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "wire": "expired",
            "ident": ["expired"],
            "payload": {"kind": "unit"}
        }));
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    assert!(diff.has_breaking_changes());
    assert_eq!(
        diff.changes
            .iter()
            .find(|change| change.path == "types.LoginError.variants.expired")
            .unwrap()
            .level,
        ChangeLevel::Breaking
    );
}

#[test]
fn object_openness_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["openness"] = serde_json::json!("reject");
    assert_eq!(level(&old, changed, "types.User"), ChangeLevel::Breaking);
}

#[test]
fn type_name_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.types.get_mut("User").unwrap().name = vec!["account".into()];
    let change = old
        .diff(&new)
        .changes
        .into_iter()
        .find(|change| change.path == "types.User")
        .unwrap();
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "type name changed");
}

#[test]
fn idempotency_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.operations[0].semantics.idempotent = true;
    let change = old
        .diff(&new)
        .changes
        .into_iter()
        .find(|change| change.path == "operations.auth.login.semantics")
        .unwrap();
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "operation idempotency changed");
}

#[test]
fn route_changes_with_stable_operation_identity_are_nonbreaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["operations"][0]["path"][1]["lit"] = "sign-in".into();
    assert_eq!(
        level(&old, changed, "operations.auth.login"),
        ChangeLevel::NonBreaking
    );
}

#[test]
fn path_parameter_type_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["operations"][1]["path"][1]["ty"] =
        serde_json::json!({"kind": "int", "bits": 32, "signed": true, "wire": "number"});
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.events.watch.path[0]" && change.level == ChangeLevel::Breaking
    }));
    assert!(!diff.changes.iter().any(|change| {
        change.path == "operations.events.watch" && change.detail == "route URL changed"
    }));
}

#[test]
fn change_paths_keep_readable_operation_ids() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["operations"][0]["id"] = serde_json::json!([["auth"], ["sign_in"]]);
    changed["operations"][1]["id"] = serde_json::json!([["events.v2"], ["watch_all"]]);
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    for change in &diff.changes {
        assert!(
            !change.path.contains('%'),
            "change path {} escapes the operation id",
            change.path
        );
    }
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.auth.sign_in" && change.detail == "operation added"
    }));
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.events.v2.watch_all" && change.detail == "operation added"
    }));
    assert_eq!(crate::operation_path(&[vec!["a.b".into()]]), "a%2Eb");
}

#[test]
fn diff_operation_paths_are_injective() {
    let mut old: Document = serde_json::from_value(fixture()).unwrap();
    old.operations[0].id = vec![vec!["a".into(), "b".into()]];
    let mut collision = old.operations[0].clone();
    collision.id = vec![vec!["a_b".into()]];
    old.operations.push(collision);
    old.validate().unwrap();

    let mut new = old.clone();
    new.operations.clear();
    new.validate().unwrap();

    let paths = old
        .diff(&new)
        .changes
        .into_iter()
        .filter(|change| change.detail == "operation removed")
        .map(|change| change.path)
        .collect::<Vec<_>>();
    assert!(paths.contains(&"operations.a_b".into()));
    assert!(paths.contains(&"operations.a%5Fb".into()));
}

#[test]
fn opaque_metadata_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["fields"][0]["ty"] = serde_json::json!({
        "kind": "opaque",
        "codec": "example",
        "wire": "json"
    });
    let new: Document = serde_json::from_value(changed).unwrap();
    let diff = old.diff(&new);
    assert!(diff.changes.iter().any(|change| {
        change.path == "types.User.fields.id" && change.level == ChangeLevel::Breaking
    }));
}

#[test]
fn new_errors_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut operation = old.clone();
    let mut operation_error = operation.operations[0].errors[0].clone();
    operation_error.status = crate::ErrorStatus::Exact(418);
    operation_error.name = vec!["teapot".into()];
    operation.operations[0].errors.push(operation_error);
    let diff = old.diff(&operation);
    assert!(diff.has_breaking_changes());
    assert!(diff.changes.iter().any(|change| {
        change.path == "operations.auth.login.errors.teapot"
            && change.level == ChangeLevel::Breaking
    }));

    let mut common = old.clone();
    let mut common_error = common.operations[0].errors[0].clone();
    common_error.status = crate::ErrorStatus::Exact(503);
    common_error.name = vec!["unavailable".into()];
    common.common_errors.push(common_error);
    let diff = old.diff(&common);
    assert!(diff.has_breaking_changes());
    assert!(diff.changes.iter().any(|change| {
        change.path == "common_errors.errors.unavailable" && change.level == ChangeLevel::Breaking
    }));
}

#[test]
fn added_success_response_statuses_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut created = old.clone();
    let mut accepted = created.operations[0].responses[0].clone();
    accepted.status = crate::ResponseStatus::Exact(201);
    created.operations[0].responses.push(accepted);
    created.validate().unwrap();

    let diff = old.diff(&created);
    assert!(diff.has_breaking_changes());
    let change = change_at(&old, &created, "operations.auth.login.responses.201");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "response status added");
}

#[test]
fn request_body_requiredness_is_classified() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut optional = old.clone();
    optional.operations[0].body.as_mut().unwrap().required = false;
    let change = old
        .diff(&optional)
        .changes
        .into_iter()
        .find(|change| change.path == "operations.auth.login.body")
        .unwrap();
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "request body became optional");

    let mut required = optional.clone();
    required.operations[0].body.as_mut().unwrap().required = true;
    assert_eq!(
        optional
            .diff(&required)
            .changes
            .into_iter()
            .find(|change| change.path == "operations.auth.login.body")
            .unwrap()
            .level,
        ChangeLevel::Breaking
    );

    let mut media = old.clone();
    media.operations[0].body.as_mut().unwrap().media_type =
        Some("application/vnd.acme+json".into());
    assert_eq!(
        old.diff(&media)
            .changes
            .into_iter()
            .find(|change| change.path == "operations.auth.login.body")
            .unwrap()
            .level,
        ChangeLevel::Breaking
    );
}

#[test]
fn document_info_changes_are_classified() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.info.title = "Renamed".into();
    new.info.version = "2".into();
    new.info.description = Some("now with words".into());
    new.info.servers = vec!["https://api.example.test/v2".into()];
    new.info.ext.insert(
        "test.example.source/v1".into(),
        serde_json::json!({"kept": false}),
    );
    new.ext.insert(
        "com.example.cursor/v1".into(),
        serde_json::json!({"page": 2}),
    );

    let diff = old.diff(&new);
    for (path, level, detail) in [
        ("info.title", ChangeLevel::Info, "API title changed"),
        (
            "info.version",
            ChangeLevel::NonBreaking,
            "API version changed",
        ),
        (
            "info.description",
            ChangeLevel::Info,
            "API description changed",
        ),
        (
            "info.servers",
            ChangeLevel::NonBreaking,
            "API servers changed",
        ),
        (
            "info.ext",
            ChangeLevel::Warning,
            "document metadata changed",
        ),
        ("ext", ChangeLevel::Warning, "document metadata changed"),
    ] {
        let change = diff
            .changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no change reported at {path}"));
        assert_eq!((change.level, change.detail.as_str()), (level, detail));
    }
}

#[test]
fn opaque_type_metadata_is_reported_as_a_warning() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut changed = fixture();
    changed["types"]["User"]["ext"]["test.example.source/v1"] = serde_json::json!("dropped");
    assert_eq!(level(&old, changed, "types.User"), ChangeLevel::Warning);

    let mut changed = fixture();
    changed["types"]["User"]["fields"][0]["ext"]["test.example.source/v1"] = serde_json::json!(2);
    assert_eq!(
        level(&old, changed, "types.User.fields.id"),
        ChangeLevel::Warning
    );

    let mut changed = fixture();
    changed["types"]["LoginError"]["variants"][1]["ext"]["test.example.source/v1"] =
        serde_json::json!(true);
    assert_eq!(
        level(&old, changed, "types.LoginError.variants.locked"),
        ChangeLevel::Warning
    );
}

#[test]
fn enum_variant_identifier_changes_are_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    let TypeKind::Enum { variants, .. } = &mut new.types.get_mut("LoginError").unwrap().definition
    else {
        unreachable!()
    };
    variants[1].ident = vec!["locked".into(), "out".into()];
    let change = change_at(&old, &new, "types.LoginError.variants.locked");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "enum variant identifier changed");
}

#[test]
fn field_default_changes_follow_input_direction() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut input = fixture();
    input["types"]["LoginRequest"]["fields"][0]["default"] =
        serde_json::json!("someone@example.test");
    assert_eq!(
        level(&old, input, "types.LoginRequest.fields.email"),
        ChangeLevel::Breaking
    );

    let mut output = fixture();
    output["types"]["User"]["fields"][1]["default"] = serde_json::json!("anonymous");
    assert_eq!(
        level(&old, output, "types.User.fields.nickname"),
        ChangeLevel::NonBreaking
    );
}

#[test]
fn both_direction_presence_changes_remain_breaking() {
    let mut old = fixture();
    old["types"]["User"]["direction"] = serde_json::json!("both");
    old["types"]["User"]["fields"][1]["presence"] = serde_json::json!("required");
    let old: Document = serde_json::from_value(old).unwrap();
    let mut changed = serde_json::to_value(&old).unwrap();
    changed["types"]["User"]["fields"][1]["presence"] = serde_json::json!("optional");
    let new: Document = serde_json::from_value(changed).unwrap();
    assert_eq!(
        change_at(&old, &new, "types.User.fields.nickname").level,
        ChangeLevel::Breaking
    );
}

#[test]
fn operation_safety_is_classified() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut safe = old.clone();
    safe.operations[0].semantics.safe = true;
    let change = change_at(&old, &safe, "operations.auth.login.semantics");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "operation safety changed");
}

#[test]
fn operation_metadata_changes_are_reported_as_a_warning() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut new = old.clone();
    new.operations[0].ext.insert(
        "test.example.source/v1".into(),
        serde_json::json!("dropped"),
    );
    let change = change_at(&old, &new, "operations.auth.login.ext");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "operation metadata changed");
}

fn doc(summary: Option<&str>, deprecated: Option<bool>) -> Doc {
    Doc {
        summary: summary.map(str::to_owned),
        description: None,
        deprecated,
        tags: Vec::new(),
    }
}

#[test]
fn auth_scheme_documentation_changes_are_not_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut reformatted = old.clone();
    reformatted.auth_schemes.insert(
        "session".into(),
        AuthScheme::Bearer {
            format: Some("opaque".into()),
        },
    );
    let change = change_at(&old, &reformatted, "auth_schemes.session");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "bearer token format documentation changed");
    assert!(!old.diff(&reformatted).has_breaking_changes());

    let described = {
        let mut document = old.clone();
        document.auth_schemes.insert(
            "custom".into(),
            AuthScheme::Custom {
                description: "first".into(),
                adapter: None,
                ext: BTreeMap::new(),
            },
        );
        document
    };
    let mut reworded = described.clone();
    reworded.auth_schemes.insert(
        "custom".into(),
        AuthScheme::Custom {
            description: "second".into(),
            adapter: None,
            ext: BTreeMap::new(),
        },
    );
    let change = change_at(&described, &reworded, "auth_schemes.custom");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "custom scheme description changed");

    let mut extended = described.clone();
    let AuthScheme::Custom { ext, .. } = extended.auth_schemes.get_mut("custom").unwrap() else {
        unreachable!()
    };
    *ext = BTreeMap::from([("test.example.source/v1".into(), serde_json::json!(1))]);
    let change = change_at(&described, &extended, "auth_schemes.custom");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "custom scheme metadata changed");
    assert!(!described.diff(&extended).has_breaking_changes());
}

#[test]
fn auth_scheme_credential_carriers_stay_breaking() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    for (scheme, detail) in [
        (
            AuthScheme::ApiKey {
                location: crate::ApiKeyLocation::Header,
                name: "X-Account".into(),
            },
            "API key name changed",
        ),
        (
            AuthScheme::ApiKey {
                location: crate::ApiKeyLocation::Query,
                name: "X-Tenant".into(),
            },
            "API key location changed",
        ),
        (AuthScheme::Basic, "authentication scheme kind changed"),
    ] {
        let mut new = old.clone();
        new.auth_schemes.insert("tenant".into(), scheme.clone());
        new.auth_schemes.insert("session".into(), scheme);
        let path = if detail.starts_with("API key") {
            "auth_schemes.tenant"
        } else {
            "auth_schemes.session"
        };
        let change = change_at(&old, &new, path);
        assert_eq!(change.level, ChangeLevel::Breaking, "{detail}");
        assert_eq!(change.detail, detail);
    }

    let with_cookie = {
        let mut document = old.clone();
        document.auth_schemes.insert(
            "browser".into(),
            AuthScheme::CookieSession {
                cookie: "sid".into(),
            },
        );
        document
    };
    let mut renamed = with_cookie.clone();
    renamed.auth_schemes.insert(
        "browser".into(),
        AuthScheme::CookieSession {
            cookie: "session_id".into(),
        },
    );
    let change = change_at(&with_cookie, &renamed, "auth_schemes.browser");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "session cookie name changed");

    let mut removed = with_cookie.clone();
    removed.auth_schemes.remove("browser");
    let change = change_at(&with_cookie, &removed, "auth_schemes.browser");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "authentication scheme removed");
}

#[test]
fn oauth_flow_changes_are_classified() {
    let old = {
        let mut document: Document = serde_json::from_value(fixture()).unwrap();
        document.auth_schemes.insert(
            "oauth".into(),
            AuthScheme::OAuth2 {
                flows: vec![OAuthFlow {
                    kind: crate::OAuthFlowKind::ClientCredentials,
                    authorization_url: None,
                    token_url: Some("https://identity.example.test/token".into()),
                    scopes: BTreeMap::new(),
                    ext: BTreeMap::new(),
                }],
            },
        );
        document
    };
    const PATH: &str = "auth_schemes.oauth.flows.ClientCredentials";

    let mut retokenized = old.clone();
    let AuthScheme::OAuth2 { flows } = retokenized.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows[0].token_url = Some("https://identity.example.test/v2/token".into());
    let change = change_at(&old, &retokenized, PATH);
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "OAuth2 token URL changed");

    let mut documented = old.clone();
    let AuthScheme::OAuth2 { flows } = documented.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows[0]
        .scopes
        .insert("read".into(), "read everything".into());
    let change = change_at(&old, &documented, PATH);
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "OAuth2 scope documentation changed");
    assert!(!old.diff(&documented).has_breaking_changes());

    let mut extended = old.clone();
    let AuthScheme::OAuth2 { flows } = extended.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows[0]
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(true));
    let change = change_at(&old, &extended, PATH);
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "OAuth2 flow metadata changed");

    let mut added = old.clone();
    let AuthScheme::OAuth2 { flows } = added.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows.push(OAuthFlow {
        kind: crate::OAuthFlowKind::Password,
        authorization_url: None,
        token_url: Some("https://identity.example.test/token".into()),
        scopes: BTreeMap::new(),
        ext: BTreeMap::new(),
    });
    let change = change_at(&old, &added, "auth_schemes.oauth.flows.Password");
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "OAuth2 flow added");

    let mut removed = added.clone();
    let AuthScheme::OAuth2 { flows } = removed.auth_schemes.get_mut("oauth").unwrap() else {
        unreachable!()
    };
    flows.retain(|flow| flow.kind != crate::OAuthFlowKind::Password);
    let change = change_at(&added, &removed, "auth_schemes.oauth.flows.Password");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "OAuth2 flow removed");
}

#[test]
fn response_metadata_changes_follow_metadata_levels() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut extended = old.clone();
    extended.operations[0].responses[0].ext.insert(
        "test.example.source/v1".into(),
        serde_json::json!("dropped"),
    );
    let change = change_at(&old, &extended, "operations.auth.login.responses.200");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "response metadata changed");
    assert!(!old.diff(&extended).has_breaking_changes());

    let mut retyped = old.clone();
    retyped.operations[0].responses[0].body = Some(TypeRef::Named(crate::NamedRef {
        id: "LoginRequest".into(),
        variant: None,
    }));
    let change = change_at(&old, &retyped, "operations.auth.login.responses.200");
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "response changed");
}

#[test]
fn error_documentation_and_metadata_follow_metadata_levels() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    const PATH: &str = "operations.auth.login.errors.login_error";

    let mut summarized = old.clone();
    summarized.operations[0].errors[0].doc = Some(doc(Some("Login failed"), None));
    let change = change_at(&old, &summarized, PATH);
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "error documentation changed");
    assert!(!old.diff(&summarized).has_breaking_changes());

    let mut deprecated = summarized.clone();
    deprecated.operations[0].errors[0].doc = Some(doc(Some("Login failed"), Some(true)));
    let change = change_at(&summarized, &deprecated, PATH);
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "error deprecation changed");
    assert!(!summarized.diff(&deprecated).has_breaking_changes());

    let mut extended = old.clone();
    extended.operations[0].errors[0]
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    let change = change_at(&old, &extended, PATH);
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "error metadata changed");
    assert!(!old.diff(&extended).has_breaking_changes());

    let mut retyped = old.clone();
    retyped.operations[0].errors[0].body = Some(TypeRef::Named(crate::NamedRef {
        id: "LoginRequest".into(),
        variant: None,
    }));
    let change = change_at(&old, &retyped, PATH);
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "error definition changed");
}

#[test]
fn request_header_metadata_changes_are_reported() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut extended = old.clone();
    extended.operations[1].headers[0]
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    let change = change_at(
        &old,
        &extended,
        "operations.events.watch.headers.X-Request-ID",
    );
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "request header metadata changed");
    assert!(!old.diff(&extended).has_breaking_changes());

    let mut retyped = old.clone();
    retyped.operations[1].headers[0].ty = TypeRef::Inline(InlineType::Int {
        bits: 32,
        signed: true,
        wire: crate::IntegerWire::Number,
        minimum: None,
        maximum: None,
        multiple_of: None,
    });
    retyped.operations[1].headers[0]
        .ext
        .insert("test.example.source/v1".into(), serde_json::json!(1));
    let path = "operations.events.watch.headers.X-Request-ID";
    let diff = old.diff(&retyped);
    assert!(diff.has_breaking_changes());
    assert!(
        diff.changes.iter().any(|change| {
            change.path == path && change.detail == "request header type changed"
        })
    );
    assert!(diff.changes.iter().any(|change| {
        change.path == path && change.detail == "request header metadata changed"
    }));
}

#[test]
fn operation_documentation_and_deprecation_are_reported() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut documented = old.clone();
    documented.operations[0].doc = Some(doc(Some("Sign in"), None));
    let change = change_at(&old, &documented, "operations.auth.login.doc");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "operation documentation changed");
    assert!(!old.diff(&documented).has_breaking_changes());

    let mut deprecated = documented.clone();
    deprecated.operations[0].doc = Some(doc(Some("Sign in"), Some(true)));
    let change = change_at(&documented, &deprecated, "operations.auth.login.doc");
    assert_eq!(change.level, ChangeLevel::Warning);
    assert_eq!(change.detail, "operation deprecation changed");
    assert!(!documented.diff(&deprecated).has_breaking_changes());

    let mut unannotated = documented.clone();
    unannotated.operations[0].doc = None;
    let change = change_at(&documented, &unannotated, "operations.auth.login.doc");
    assert_eq!(change.level, ChangeLevel::Info);
    assert_eq!(change.detail, "operation documentation changed");
}

#[test]
fn pagination_descriptors_are_classified_when_added_removed_or_rewritten() {
    let old: Document = serde_json::from_value(fixture()).unwrap();
    let mut added = old.clone();
    let operation = added
        .operations
        .iter_mut()
        .find(|operation| operation.query.is_some())
        .unwrap();
    operation.semantics.pagination = Some(
        serde_json::from_value(serde_json::json!({
            "request": {"kind": "cursor", "field": "limit"},
            "items": "/items",
            "next": "/next"
        }))
        .unwrap(),
    );
    added.validate().unwrap();

    let change = change_at(
        &old,
        &added,
        "operations.events.watch.semantics.pagination",
    );
    assert_eq!(change.level, ChangeLevel::NonBreaking);
    assert_eq!(change.detail, "pagination descriptor added");

    let change = change_at(
        &added,
        &old,
        "operations.events.watch.semantics.pagination",
    );
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "pagination descriptor removed");

    let mut rewritten = added.clone();
    let operation = rewritten
        .operations
        .iter_mut()
        .find(|operation| operation.query.is_some())
        .unwrap();
    operation.semantics.pagination.as_mut().unwrap().request =
        serde_json::from_value(serde_json::json!({"kind": "offset", "field": "limit"})).unwrap();
    let change = change_at(
        &added,
        &rewritten,
        "operations.events.watch.semantics.pagination",
    );
    assert_eq!(change.level, ChangeLevel::Breaking);
    assert_eq!(change.detail, "pagination request encoding changed");
}
