use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Access, CoverageLevel, Direction, Doc, Document, EnumRepr, Field, FieldFormat, InlineType,
    IntegerWire, ObjectOpenness, Presence, TypeDef, TypeKind, TypeRef, ValidationCoverage, Variant,
    VariantPayload, type_subject,
};

use super::backend::{
    Backend, Binding, Capability, Declaration, DeclarationKind, FloatShape, IntCarrier,
    IntegerShape, Member, ObjectShape, Rendered, StringCarrier, StringShape,
};
use crate::naming::{camel, pascal, unique_names};
use crate::{DateMode, EmitError, EmitMode, FieldCase, Int64, TsOptions};

struct Inline {
    name: String,
    expression: String,
}

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

    fn inline_validator<B: Backend>(
        &mut self,
        backend: &mut B,
        reference: &TypeRef,
        subject: &str,
    ) -> Result<String, EmitError> {
        let json = crate::client::type_ref_json(reference, self.options)?;
        if let Some(inline) = self.inlines.get(&json) {
            return Ok(inline.name.clone());
        }
        let name = loop {
            self.inline_count += 1;
            let candidate = format!("validateInline{}", self.inline_count);
            if self.claimed.insert(candidate.clone()) {
                break candidate;
            }
        };
        let expression = self.in_subject(subject, |walk| walk.reference(backend, reference))?;
        self.inlines.insert(
            json,
            Inline {
                name: name.clone(),
                expression,
            },
        );
        Ok(name)
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

    fn reference<B: Backend>(
        &mut self,
        backend: &mut B,
        reference: &TypeRef,
    ) -> Result<String, EmitError> {
        if let TypeRef::Named(named) = reference {
            let name = self
                .declaration_name(&named.id, named.variant.as_deref())?
                .to_owned();
            let rendered = backend.reference(&name, named.variant.as_deref());
            self.dependencies.push(name);
            return self.record(backend.name(), rendered);
        }
        self.inline(backend, reference)
    }

    fn inline<B: Backend>(
        &mut self,
        backend: &mut B,
        reference: &TypeRef,
    ) -> Result<String, EmitError> {
        let TypeRef::Inline(inline) = reference else {
            unreachable!("a reference that is not inline is named")
        };
        let rendered = match inline {
            InlineType::Bool => backend.boolean(),
            InlineType::Null => backend.null(),
            InlineType::Any => backend.any(),
            InlineType::Opaque { .. } => backend.unknown(),
            InlineType::Nullable { inner } => {
                let inner = self.reference(backend, inner)?;
                backend.nullable(&inner)
            }
            InlineType::Int {
                bits,
                signed,
                wire,
                minimum,
                maximum,
                multiple_of,
            } => backend.integer(&IntegerShape {
                carrier: self.int_carrier(*wire, *bits),
                bits: *bits,
                signed: *signed,
                minimum: minimum.as_ref(),
                maximum: maximum.as_ref(),
                multiple_of: multiple_of.as_ref(),
            })?,
            InlineType::Float {
                bits,
                minimum,
                maximum,
                exclusive_minimum,
                exclusive_maximum,
                multiple_of,
            } => backend.float(&FloatShape {
                bits: *bits,
                minimum: minimum.as_ref(),
                maximum: maximum.as_ref(),
                exclusive_minimum: exclusive_minimum.as_ref(),
                exclusive_maximum: exclusive_maximum.as_ref(),
                multiple_of: multiple_of.as_ref(),
            })?,
            InlineType::Decimal { wire } => backend.decimal(*wire),
            InlineType::String {
                format,
                min_length,
                max_length,
                pattern,
            } => backend.string(&StringShape {
                format: *format,
                carrier: self.string_carrier(*format),
                min_length: *min_length,
                max_length: *max_length,
                pattern: pattern.as_deref(),
            }),
            InlineType::Bytes { .. } => backend.bytes(),
            InlineType::Array { item, unique } => {
                let item = self.reference(backend, item)?;
                backend.array(&item, *unique == Some(true))
            }
            InlineType::Map { key, value } => {
                let key = self.reference(backend, key)?;
                let value = self.reference(backend, value)?;
                backend.map(&key, &value)
            }
            InlineType::Tuple { items } => {
                let items = self.items(backend, items)?;
                backend.tuple(&items)
            }
            InlineType::Struct {
                fields,
                flattened,
                openness,
            } => {
                return self.structure(backend, fields, flattened, openness, Direction::Both);
            }
        };
        self.record(backend.name(), rendered)
    }

    fn items<B: Backend>(
        &mut self,
        backend: &mut B,
        items: &[TypeRef],
    ) -> Result<Vec<String>, EmitError> {
        items
            .iter()
            .map(|item| self.reference(backend, item))
            .collect()
    }

    fn int_carrier(&self, wire: IntegerWire, bits: u16) -> IntCarrier {
        if wire == IntegerWire::String {
            return IntCarrier::Text;
        }
        if bits > 53 {
            return match self.options.int64 {
                Int64::String => IntCarrier::Text,
                Int64::Bigint if self.options.mode == EmitMode::Strict => IntCarrier::Bigint,
                Int64::Bigint | Int64::Number => IntCarrier::Number,
            };
        }
        IntCarrier::Number
    }

    fn string_carrier(&self, format: Option<FieldFormat>) -> StringCarrier {
        if format == Some(FieldFormat::DateTime) && self.options.date_mode == DateMode::Date {
            StringCarrier::Date
        } else {
            StringCarrier::Text
        }
    }

    fn flattened(&self, members: &[TypeRef]) -> Vec<Flattened<'a>> {
        let mut found = Vec::new();
        let mut seen = BTreeSet::new();
        for member in members {
            let key = match member {
                TypeRef::Named(named) => named.id.clone(),
                TypeRef::Inline(_) => String::new(),
            };
            if seen.insert(key) {
                found.push(self.flattened_member(member));
            }
        }
        found
    }

    fn inlined(&self, reference: &TypeRef) -> Option<&'a TypeDef> {
        match self.flattened_member(reference) {
            Flattened::Struct(definition) => Some(definition),
            _ => None,
        }
    }

    fn flattened_member(&self, reference: &TypeRef) -> Flattened<'a> {
        let document = self.document;
        let mut current = reference;
        let mut seen = BTreeSet::new();
        loop {
            match current {
                TypeRef::Inline(InlineType::Opaque { .. }) => return Flattened::Opaque,
                TypeRef::Inline(_) => return Flattened::Other,
                TypeRef::Named(named) => {
                    if named.variant.is_some() || !seen.insert(named.id.clone()) {
                        return Flattened::Other;
                    }
                    let Some(definition) = document.types.get(&named.id) else {
                        return Flattened::Other;
                    };
                    match &definition.definition {
                        TypeKind::Newtype { inner } => current = inner,
                        TypeKind::Struct { .. } => return Flattened::Struct(definition),
                        TypeKind::Enum { .. } => return Flattened::Other,
                    }
                }
            }
        }
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

enum Flattened<'a> {
    Struct(&'a TypeDef),
    Opaque,
    Other,
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

fn openness_value<'a>(ty: &'a TypeRef, document: &'a Document) -> &'a TypeRef {
    let mut current = ty;
    let mut visited = BTreeSet::new();
    loop {
        match current {
            TypeRef::Inline(InlineType::Map { value, .. }) => return value,
            TypeRef::Named(named) if visited.insert(named.id.clone()) => {
                let Some(TypeKind::Newtype { inner }) = document
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                else {
                    return current;
                };
                current = inner;
            }
            _ => return current,
        }
    }
}

pub(crate) fn declaration_names(
    document: &Document,
    names: &BTreeMap<String, String>,
) -> BTreeMap<(String, Option<String>), String> {
    let mut keys = Vec::new();
    let mut bases = Vec::new();
    for (id, definition) in &document.types {
        let name = &names[id];
        if let TypeKind::Enum { variants, .. } = &definition.definition {
            for variant in variants {
                keys.push((id.clone(), Some(variant.wire.clone())));
                bases.push(format!("{name}{}", pascal(&variant.ident)));
            }
        }
        keys.push((id.clone(), None));
        bases.push(name.clone());
    }
    keys.into_iter().zip(unique_names(bases, &[])).collect()
}

pub(crate) fn validator_names(
    document: &Document,
    declarations: &BTreeMap<(String, Option<String>), String>,
    reserved: &[&str],
) -> BTreeMap<(String, Option<String>), String> {
    let mut keys = Vec::new();
    let mut bases = Vec::new();
    for (key, name) in declarations {
        let input_only = document
            .types
            .get(&key.0)
            .is_some_and(|definition| definition.direction == Direction::Input);
        if input_only {
            continue;
        }
        keys.push(key.clone());
        bases.push(format!("validate{name}"));
    }
    keys.into_iter()
        .zip(unique_names(bases, reserved))
        .collect()
}
