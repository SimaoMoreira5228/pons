mod flattened;
mod types;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Access, CoverageLevel, Direction, Doc, Document, EnumRepr, Field, InlineType, ObjectOpenness,
    Presence, TypeDef, TypeKind, TypeRef, ValidationCoverage, Variant, VariantPayload,
    type_subject,
};

use self::flattened::{Flattened, openness_value};
use self::types::Inline;
use super::backend::{
    Backend, Binding, Capability, Declaration, DeclarationKind, Member, ObjectShape, Rendered,
};
use super::naming::{declaration_names, validator_names};
use crate::naming::camel;
use crate::{EmitError, FieldCase, TsOptions};

struct OwnedDeclaration<'a> {
    id: String,
    variant: Option<String>,
    name: String,
    reference: String,
    schema: String,
    doc: Option<&'a Doc>,
    dependencies: Vec<String>,
    kind: DeclarationKind,
}

#[derive(Default)]
struct RecordedCapabilities {
    widened: BTreeSet<Capability>,
    unsupported: BTreeSet<Capability>,
}

pub(crate) struct Walk<'a> {
    document: &'a Document,
    options: &'a TsOptions,
    declarations: BTreeMap<(String, Option<String>), String>,
    validators: BTreeMap<(String, Option<String>), String>,
    claimed: BTreeSet<String>,
    inlines: BTreeMap<String, Inline>,
    inline_count: usize,
    capabilities: BTreeMap<String, RecordedCapabilities>,
    subject: Option<String>,
    dependencies: Vec<String>,
    owned: Vec<OwnedDeclaration<'a>>,
    built: bool,
}

impl<'a> Walk<'a> {
    pub(crate) fn new(
        document: &'a Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
        reserved: &[&str],
    ) -> Self {
        let declarations = declaration_names(document, names);
        let validators = validator_names(document, &declarations, reserved);
        let claimed = validators.values().cloned().collect();
        Walk {
            document,
            options,
            declarations,
            validators,
            claimed,
            inlines: BTreeMap::new(),
            inline_count: 0,
            capabilities: BTreeMap::new(),
            subject: None,
            dependencies: Vec::new(),
            owned: Vec::new(),
            built: false,
        }
    }

    pub(crate) fn validate<B: Backend>(
        &mut self,
        backend: &mut B,
        reference: &TypeRef,
        subject: &str,
    ) -> Result<String, EmitError> {
        match reference {
            TypeRef::Named(named) => self.validator(&named.id, named.variant.as_deref()),
            TypeRef::Inline(_) => self.inline_validator(backend, reference, subject),
        }
    }

    pub(crate) fn declarations<B: Backend>(&mut self, backend: &mut B) -> Result<(), EmitError> {
        if self.built {
            return Ok(());
        }
        let document = self.document;
        for (id, definition) in &document.types {
            if definition.direction == Direction::Input {
                continue;
            }
            let subject = type_subject(id);
            self.in_subject(&subject, |walk| walk.declare(backend, id, definition))?;
        }
        self.built = true;
        Ok(())
    }

    pub(crate) fn module<B: Backend>(&self, backend: &mut B) -> Result<String, EmitError> {
        let declarations = self
            .owned
            .iter()
            .map(|declaration| Declaration {
                name: &declaration.name,
                schema: &declaration.schema,
                doc: declaration.doc,
                dependencies: &declaration.dependencies,
                kind: declaration.kind,
            })
            .collect::<Vec<_>>();
        backend.module(&declarations)
    }

    pub(crate) fn bindings(&self) -> Result<Vec<Binding>, EmitError> {
        let mut bindings = Vec::new();
        for declaration in &self.owned {
            bindings.push(Binding {
                name: self
                    .validator(&declaration.id, declaration.variant.as_deref())?
                    .to_owned(),
                schema: declaration.reference.clone(),
            });
        }
        for inline in self.inlines.values() {
            bindings.push(Binding {
                name: inline.name.clone(),
                schema: inline.expression.clone(),
            });
        }
        Ok(bindings)
    }

    pub(crate) fn has_bindings(&self) -> bool {
        !self.validators.is_empty() || !self.inlines.is_empty()
    }

    pub(crate) fn declares(&self, name: &str) -> bool {
        self.declarations.values().any(|declared| declared == name)
    }

    pub(crate) fn coverage(&self) -> ValidationCoverage {
        let seeds = self
            .capabilities
            .iter()
            .filter_map(|(subject, recorded)| {
                let level = if !recorded.unsupported.is_empty() {
                    CoverageLevel::Unsupported
                } else if !recorded.widened.is_empty() {
                    CoverageLevel::Lossy
                } else {
                    return None;
                };
                Some((subject.clone(), level))
            })
            .collect();
        super::coverage::propagate(self.document, &seeds)
    }

    fn in_subject<T>(
        &mut self,
        subject: &str,
        body: impl FnOnce(&mut Self) -> Result<T, EmitError>,
    ) -> Result<T, EmitError> {
        let previous = self.subject.replace(subject.to_owned());
        let result = body(self);
        self.subject = previous;
        result
    }

    fn record(&mut self, backend: &'static str, rendered: Rendered) -> Result<String, EmitError> {
        match rendered {
            Rendered::Exact(schema) => Ok(schema),
            Rendered::Widened(schema, capability) => {
                self.record_capability(capability, false);
                Ok(schema)
            }
            Rendered::Unsupported(capability) => {
                self.record_capability(capability, true);
                Err(EmitError(format!(
                    "the {backend} validation backend cannot check {}, because it does not support {}",
                    self.subject.as_deref().unwrap_or("the document"),
                    capability.id()
                )))
            }
        }
    }

    fn record_capability(&mut self, capability: Capability, unsupported: bool) {
        let Some(subject) = self.subject.as_ref() else {
            return;
        };
        let recorded = self.capabilities.entry(subject.clone()).or_default();
        if unsupported {
            recorded.unsupported.insert(capability);
        } else {
            recorded.widened.insert(capability);
        }
    }

    fn declaration_name(&self, id: &str, variant: Option<&str>) -> Result<&str, EmitError> {
        self.declarations
            .get(&(id.to_owned(), variant.map(str::to_owned)))
            .map(String::as_str)
            .ok_or_else(|| match variant {
                Some(variant) => EmitError(format!("unknown enum variant {variant} of type {id}")),
                None => EmitError(format!("unknown validator for type {id}")),
            })
    }

    fn validator(&self, id: &str, variant: Option<&str>) -> Result<String, EmitError> {
        self.validators
            .get(&(id.to_owned(), variant.map(str::to_owned)))
            .cloned()
            .ok_or_else(|| match variant {
                Some(variant) => EmitError(format!("unknown enum variant {variant} of type {id}")),
                None => EmitError(format!("unknown validator for type {id}")),
            })
    }

    fn declare<B: Backend>(
        &mut self,
        backend: &mut B,
        id: &str,
        definition: &'a TypeDef,
    ) -> Result<(), EmitError> {
        match &definition.definition {
            TypeKind::Enum { repr, variants } => {
                for variant in variants {
                    let name = self.declaration_name(id, Some(&variant.wire))?.to_owned();
                    let schema = self.variant(backend, id, definition, variant)?;
                    let reference = backend.schema(&name);
                    let dependencies = std::mem::take(&mut self.dependencies);
                    self.owned.push(OwnedDeclaration {
                        id: id.to_owned(),
                        variant: Some(variant.wire.clone()),
                        name,
                        reference,
                        schema,
                        doc: variant.doc.as_ref(),
                        dependencies,
                        kind: DeclarationKind::Variant,
                    });
                }
                let name = self.declaration_name(id, None)?.to_owned();
                let mut dependencies = Vec::new();
                let mut members = Vec::new();
                for variant in variants {
                    let variant_name = self.declaration_name(id, Some(&variant.wire))?;
                    dependencies.push(variant_name.to_owned());
                    members.push(backend.schema(variant_name));
                }
                let schema = match repr {
                    EnumRepr::Internal { tag } | EnumRepr::Adjacent { tag, .. } => {
                        backend.tagged_union(tag, &members)
                    }
                    _ => backend.union(&members),
                };
                let schema = self.record(backend.name(), schema)?;
                let reference = backend.schema(&name);
                self.owned.push(OwnedDeclaration {
                    id: id.to_owned(),
                    variant: None,
                    name,
                    reference,
                    schema,
                    doc: definition.doc.as_ref(),
                    dependencies,
                    kind: DeclarationKind::Type,
                });
            }
            _ => {
                let name = self.declaration_name(id, None)?.to_owned();
                let schema = self.definition(backend, id, definition)?;
                let reference = backend.schema(&name);
                let dependencies = std::mem::take(&mut self.dependencies);
                self.owned.push(OwnedDeclaration {
                    id: id.to_owned(),
                    variant: None,
                    name,
                    reference,
                    schema,
                    doc: definition.doc.as_ref(),
                    dependencies,
                    kind: DeclarationKind::Type,
                });
            }
        }
        Ok(())
    }

    fn definition<B: Backend>(
        &mut self,
        backend: &mut B,
        id: &str,
        definition: &'a TypeDef,
    ) -> Result<String, EmitError> {
        match &definition.definition {
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => self.structure(backend, fields, flattened, openness, definition.direction),
            TypeKind::Enum { .. } => {
                let name = self.declaration_name(id, None)?.to_owned();
                let rendered = backend.reference(&name, None);
                self.dependencies.push(name);
                self.record(backend.name(), rendered)
            }
            TypeKind::Newtype { inner } => self.reference(backend, inner),
        }
    }

    fn variant<B: Backend>(
        &mut self,
        backend: &mut B,
        id: &str,
        definition: &'a TypeDef,
        variant: &'a Variant,
    ) -> Result<String, EmitError> {
        let TypeKind::Enum { repr, .. } = &definition.definition else {
            unreachable!("a variant belongs to an enum")
        };
        let direction = definition.direction;
        let payload = &variant.payload;
        let rendered = match repr {
            EnumRepr::String => match payload {
                VariantPayload::Unit => backend.literal(&variant.wire),
                _ => {
                    return Err(EmitError(format!(
                        "the type {id} spells the variant {} as a string but gives it a payload, which a string enum cannot hold",
                        variant.wire
                    )));
                }
            },
            EnumRepr::External => {
                let payload = match payload {
                    VariantPayload::Unit => None,
                    _ => Some(self.payload(backend, payload, direction)?),
                };
                backend.external(&variant.wire, payload.as_deref())
            }
            EnumRepr::Internal { tag } => {
                let mut members = vec![Member {
                    key: tag.clone(),
                    schema: self.record(backend.name(), backend.literal(&variant.wire))?,
                    presence: Presence::Required,
                    accepts_undefined: false,
                }];
                match payload {
                    VariantPayload::Unit => {}
                    VariantPayload::Struct { fields } => {
                        members.extend(self.members(backend, fields, direction)?);
                    }
                    VariantPayload::Newtype { ty } => match self.inlined(ty) {
                        Some(definition) => {
                            members.extend(self.members_of(backend, definition)?);
                        }
                        None => {
                            return Err(EmitError(
                                "TypeScript cannot represent internally tagged newtype or tuple payloads"
                                    .into(),
                            ));
                        }
                    },
                    VariantPayload::Tuple { .. } => {
                        return Err(EmitError(
                            "TypeScript cannot represent internally tagged newtype or tuple payloads"
                                .into(),
                        ));
                    }
                }
                backend.object(&members, &ObjectShape::PreserveOpaque)
            }
            EnumRepr::Adjacent { tag, content } => {
                let tag = Member {
                    key: tag.clone(),
                    schema: self.record(backend.name(), backend.literal(&variant.wire))?,
                    presence: Presence::Required,
                    accepts_undefined: false,
                };
                let members = match payload {
                    VariantPayload::Unit => vec![tag],
                    _ => {
                        let payload = self.payload(backend, payload, direction)?;
                        vec![
                            tag,
                            Member {
                                key: content.clone(),
                                schema: payload,
                                presence: Presence::Required,
                                accepts_undefined: false,
                            },
                        ]
                    }
                };
                backend.object(&members, &ObjectShape::PreserveOpaque)
            }
            EnumRepr::Untagged => return self.payload(backend, payload, direction),
        };
        self.record(backend.name(), rendered)
    }

    fn payload<B: Backend>(
        &mut self,
        backend: &mut B,
        payload: &VariantPayload,
        direction: Direction,
    ) -> Result<String, EmitError> {
        let rendered = match payload {
            VariantPayload::Unit => backend.unit(),
            VariantPayload::Newtype { ty } => return self.reference(backend, ty),
            VariantPayload::Tuple { items } => {
                let items = self.items(backend, items)?;
                backend.tuple(&items)
            }
            VariantPayload::Struct { fields } => {
                let members = self.members(backend, fields, direction)?;
                backend.object(&members, &ObjectShape::PreserveOpaque)
            }
        };
        self.record(backend.name(), rendered)
    }

    fn structure<B: Backend>(
        &mut self,
        backend: &mut B,
        fields: &[Field],
        flattened: &[TypeRef],
        openness: &ObjectOpenness,
        direction: Direction,
    ) -> Result<String, EmitError> {
        let flattened = self.flattened(flattened);
        if flattened
            .iter()
            .any(|member| matches!(member, Flattened::Opaque))
        {
            return self.record(backend.name(), backend.unknown());
        }
        let mut members = self.members(backend, fields, direction)?;
        for member in flattened {
            let Flattened::Struct(definition) = member else {
                continue;
            };
            members.extend(self.members_of(backend, definition)?);
        }
        let shape = match openness {
            ObjectOpenness::Reject => ObjectShape::Reject,
            ObjectOpenness::PreserveOpaque => ObjectShape::PreserveOpaque,
            ObjectOpenness::PreserveTyped { value } => {
                let value = openness_value(value, self.document);
                ObjectShape::PreserveTyped(self.reference(backend, value)?)
            }
        };
        let rendered = backend.object(&members, &shape);
        self.record(backend.name(), rendered)
    }

    fn members<B: Backend>(
        &mut self,
        backend: &mut B,
        fields: &[Field],
        direction: Direction,
    ) -> Result<Vec<Member>, EmitError> {
        let mut members = Vec::new();
        for field in fields {
            if !field_applies(field, direction) {
                continue;
            }
            let key = match self.options.field_case {
                FieldCase::Wire => field.wire.clone(),
                FieldCase::Camel => camel(&field.ident),
            };
            let schema = self.reference(backend, &field.ty)?;
            members.push(Member {
                key,
                schema,
                presence: field.presence,
                accepts_undefined: self.accepts_undefined(&field.ty),
            });
        }
        Ok(members)
    }

    fn members_of<B: Backend>(
        &mut self,
        backend: &mut B,
        definition: &TypeDef,
    ) -> Result<Vec<Member>, EmitError> {
        let TypeKind::Struct { fields, .. } = &definition.definition else {
            return Ok(Vec::new());
        };
        self.members(backend, fields, definition.direction)
    }

    fn accepts_undefined(&self, reference: &TypeRef) -> bool {
        match reference {
            TypeRef::Inline(InlineType::Any | InlineType::Opaque { .. }) => true,
            TypeRef::Inline(InlineType::Nullable { inner }) => self.accepts_undefined(inner),
            TypeRef::Inline(_) => false,
            TypeRef::Named(named) => {
                let Some(definition) = self.document.types.get(&named.id) else {
                    return false;
                };
                match &definition.definition {
                    TypeKind::Newtype { inner } => self.accepts_undefined(inner),
                    _ => false,
                }
            }
        }
    }
}

fn field_applies(field: &Field, direction: Direction) -> bool {
    if matches!(
        (direction, field.access),
        (Direction::Input, Some(Access::ReadOnly)) | (Direction::Output, Some(Access::WriteOnly))
    ) {
        return false;
    }
    direction == Direction::Input || field.access != Some(Access::WriteOnly)
}
