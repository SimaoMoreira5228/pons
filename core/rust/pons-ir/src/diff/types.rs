use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ContentKind, Direction, Document, Field, InlineType, Presence, TypeDef, TypeKind, TypeRef,
};

use super::annotations::{add_doc_change, diff_diagnostics};
use super::paths::Path;
use super::{ChangeLevel, Diff, add};

pub(super) fn diff_types(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    old: &Document,
    new: &Document,
) {
    let refused = refuses_required_read_only(new);
    for id in old.types.keys() {
        if !new.types.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                Path::key("types", id).claim(used),
                "type removed",
            );
        }
    }
    for (id, definition) in &new.types {
        let path = Path::key("types", id);
        let Some(previous) = old.types.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                path.claim(used),
                "type added",
            );
            continue;
        };
        diff_type(diff, used, &refused, &path, id, previous, definition);
    }
}

fn refuses_required_read_only(document: &Document) -> BTreeSet<String> {
    let mut refused = BTreeSet::new();
    for operation in &document.operations {
        let carriers = operation
            .query
            .iter()
            .chain(operation.path.iter().filter_map(|segment| match segment {
                crate::PathSegment::Param { ty, .. } => Some(ty),
                crate::PathSegment::Literal { .. } => None,
            }))
            .chain(
                operation
                    .body
                    .as_ref()
                    .filter(|body| body.content == ContentKind::Form)
                    .map(|body| &body.ty),
            );
        for ty in carriers {
            record_refusing(document, ty, &mut BTreeSet::new(), &mut refused);
        }
    }
    refused
}

fn record_refusing(
    document: &Document,
    ty: &TypeRef,
    visiting: &mut BTreeSet<String>,
    refused: &mut BTreeSet<String>,
) -> bool {
    let TypeRef::Named(named) = ty else {
        return false;
    };
    if named.variant.is_some() || !visiting.insert(named.id.clone()) {
        return false;
    }
    let refusing = match document.types.get(&named.id).map(|found| &found.definition) {
        Some(TypeKind::Struct {
            fields, flattened, ..
        }) => {
            fields.iter().any(|field| {
                field.presence == Presence::Required
                    && field.access == Some(crate::Access::ReadOnly)
            }) || flattened
                .iter()
                .any(|member| record_refusing(document, member, visiting, refused))
        }
        Some(TypeKind::Newtype { inner }) => record_refusing(document, inner, visiting, refused),
        _ => false,
    };
    visiting.remove(&named.id);
    if refusing {
        refused.insert(named.id.clone());
    }
    refusing
}

fn diff_type(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    refused: &BTreeSet<String>,
    owner: &Path,
    id: &str,
    old: &TypeDef,
    new: &TypeDef,
) {
    let path = owner.claim(used);
    if old.name != new.name {
        add(diff, ChangeLevel::Breaking, &path, "type name changed");
    }
    if old.direction != new.direction {
        add(diff, ChangeLevel::Breaking, &path, "type direction changed");
    }
    if old.ext != new.ext {
        add(diff, ChangeLevel::Warning, &path, "type metadata changed");
    }
    match (&old.definition, &new.definition) {
        (
            TypeKind::Struct {
                fields: old_fields,
                flattened: old_flattened,
                openness: old_openness,
            },
            TypeKind::Struct {
                fields: new_fields,
                flattened: new_flattened,
                openness: new_openness,
            },
        ) => {
            diff_fields(
                diff,
                used,
                owner,
                new.direction,
                refused.contains(id),
                old_fields,
                new_fields,
            );
            if old_flattened != new_flattened {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    &path,
                    "flattened types changed",
                );
            }
            if old_openness != new_openness {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    &path,
                    "object openness policy changed",
                );
            }
        }
        (
            TypeKind::Enum {
                repr: old_repr,
                variants: old_variants,
            },
            TypeKind::Enum {
                repr: new_repr,
                variants: new_variants,
            },
        ) => {
            if old_repr != new_repr {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    &path,
                    "enum representation changed",
                );
            }
            let old_variants: BTreeMap<_, _> = old_variants
                .iter()
                .map(|variant| (&variant.wire, variant))
                .collect();
            let new_variants: BTreeMap<_, _> = new_variants
                .iter()
                .map(|variant| (&variant.wire, variant))
                .collect();
            for wire in old_variants.keys() {
                if !new_variants.contains_key(wire) {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        owner.child("variants", wire).claim(used),
                        "enum variant removed",
                    );
                }
            }
            for (wire, variant) in new_variants {
                let variant_path = owner.child("variants", wire).claim(used);
                if let Some(previous) = old_variants.get(wire) {
                    if previous.payload != variant.payload {
                        add(
                            diff,
                            ChangeLevel::Breaking,
                            &variant_path,
                            "enum variant payload changed",
                        );
                    }
                    if previous.ident != variant.ident {
                        add(
                            diff,
                            ChangeLevel::Breaking,
                            &variant_path,
                            "enum variant identifier changed",
                        );
                    }
                    if previous.ext != variant.ext {
                        add(
                            diff,
                            ChangeLevel::Warning,
                            &variant_path,
                            "enum variant metadata changed",
                        );
                    }
                    add_doc_change(
                        diff,
                        &variant_path,
                        previous.doc.as_ref(),
                        variant.doc.as_ref(),
                        "enum variant",
                    );
                    diff_diagnostics(
                        diff,
                        &variant_path,
                        &previous.diagnostics,
                        &variant.diagnostics,
                    );
                } else {
                    let level = if new.direction == Direction::Input {
                        ChangeLevel::NonBreaking
                    } else {
                        ChangeLevel::Breaking
                    };
                    add(diff, level, variant_path, "enum variant added");
                }
            }
        }
        (TypeKind::Newtype { inner: old_inner }, TypeKind::Newtype { inner: new_inner }) => {
            if old_inner != new_inner {
                add(diff, ChangeLevel::Breaking, &path, "newtype value changed");
            }
        }
        _ => add(diff, ChangeLevel::Breaking, &path, "type kind changed"),
    }
    add_doc_change(diff, &path, old.doc.as_ref(), new.doc.as_ref(), "type");
    diff_diagnostics(diff, &path, &old.diagnostics, &new.diagnostics);
}

fn diff_fields(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    owner: &Path,
    direction: Direction,
    carrier: bool,
    old: &[Field],
    new: &[Field],
) {
    let sent = carrier || matches!(direction, Direction::Both);
    let old: BTreeMap<_, _> = old.iter().map(|field| (&field.wire, field)).collect();
    let new: BTreeMap<_, _> = new.iter().map(|field| (&field.wire, field)).collect();
    for wire in old.keys() {
        if !new.contains_key(wire) {
            add(
                diff,
                ChangeLevel::Breaking,
                owner.child("fields", wire).claim(used),
                "field removed",
            );
        }
    }
    for (wire, field) in new {
        let path = owner.child("fields", wire).claim(used);
        let read_only = field.access == Some(crate::Access::ReadOnly);
        let mut input = matches!(direction, Direction::Input | Direction::Both) || carrier;
        if read_only && !sent {
            input = false;
        }
        let output = matches!(direction, Direction::Output | Direction::Both)
            && field.access != Some(crate::Access::WriteOnly);
        let Some(previous) = old.get(wire) else {
            add(
                diff,
                if input && field.presence == Presence::Required {
                    ChangeLevel::Breaking
                } else {
                    ChangeLevel::NonBreaking
                },
                &path,
                "field added",
            );
            continue;
        };
        if previous.ident != field.ident {
            add(
                diff,
                ChangeLevel::Breaking,
                &path,
                "field identifier changed",
            );
        }
        if previous.presence != field.presence {
            let breaking = (input
                && previous.presence == Presence::Optional
                && field.presence == Presence::Required)
                || (output
                    && previous.presence == Presence::Required
                    && field.presence == Presence::Optional);
            add(
                diff,
                if breaking {
                    ChangeLevel::Breaking
                } else {
                    ChangeLevel::NonBreaking
                },
                &path,
                "field presence changed",
            );
        }
        if previous.ty != field.ty {
            let widens_input_nullability = input
                && !output
                && !is_nullable(&previous.ty)
                && is_nullable(&field.ty)
                && inner_type(&previous.ty) == inner_type(&field.ty);
            add(
                diff,
                if widens_input_nullability {
                    ChangeLevel::NonBreaking
                } else {
                    ChangeLevel::Breaking
                },
                &path,
                "field type changed",
            );
        }
        if previous.access != field.access {
            add(diff, ChangeLevel::Breaking, &path, "field access changed");
        }
        if previous.default != field.default {
            add(
                diff,
                if input {
                    ChangeLevel::Breaking
                } else {
                    ChangeLevel::NonBreaking
                },
                &path,
                "field default changed",
            );
        }
        if previous.ext != field.ext {
            add(diff, ChangeLevel::Warning, &path, "field metadata changed");
        }
        add_doc_change(
            diff,
            &path,
            previous.doc.as_ref(),
            field.doc.as_ref(),
            "field",
        );
        diff_diagnostics(diff, &path, &previous.diagnostics, &field.diagnostics);
    }
}

fn is_nullable(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Inline(InlineType::Nullable { .. }))
}

fn inner_type(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Inline(InlineType::Nullable { inner }) => inner,
        ty => ty,
    }
}
