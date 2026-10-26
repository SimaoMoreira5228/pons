use super::*;

fn read_only_field(presence: &str) -> serde_json::Value {
    serde_json::json!({
        "wire": "since",
        "ident": ["since"],
        "ty": {"kind": "string"},
        "presence": presence,
        "access": "read_only"
    })
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
