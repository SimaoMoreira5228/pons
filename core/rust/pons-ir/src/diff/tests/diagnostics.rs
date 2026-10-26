use super::*;

fn set_diagnostics(diagnostics: &mut Vec<Diagnostic>, severity: Option<DiagnosticSeverity>) {
    diagnostics.clear();
    diagnostics.extend(severity.map(|severity| Diagnostic {
        severity,
        code: "type.x".into(),
        message: "unresolved".into(),
        origin: None,
        fidelity: None,
        capability: None,
        ext: BTreeMap::new(),
    }));
}

type DiagnosticSite = (&'static str, fn(&mut Document, Option<DiagnosticSeverity>));

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
            fidelity: None,
            capability: None,
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
                fidelity: None,
                capability: None,
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
