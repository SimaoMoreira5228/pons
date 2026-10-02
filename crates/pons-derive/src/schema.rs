use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use serde_derive_internals::{ast, attr};
use syn::{GenericArgument, Ident, Type, parse_quote};

pub(super) fn expand(container: &ast::Container<'_>) -> TokenStream {
    let ident = &container.ident;
    let mut generics = container.generics.clone();
    let type_params: Vec<Ident> = generics
        .type_params()
        .map(|param| param.ident.clone())
        .collect();
    for ty in type_params {
        generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(#ty: ::pons_core::Schema));
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let name = words(&ident.to_string());
    let doc = doc_value(&container.original.attrs);
    let base_definition = match &container.data {
        ast::Data::Struct(style, fields) => struct_definition(*style, fields, &container.attrs),
        ast::Data::Enum(variants) => enum_definition(variants, &container.attrs),
    };
    let input_type = container
        .attrs
        .type_from()
        .or_else(|| container.attrs.type_try_from());
    let output_type = container.attrs.type_into();
    let input_definition = converted_definition(
        input_type,
        quote!(::pons_core::ir::Direction::Input),
        &base_definition,
    );
    let output_definition = converted_definition(
        output_type,
        quote!(::pons_core::ir::Direction::Output),
        &base_definition,
    );
    let both_type = input_type.filter(|input| {
        output_type.is_some_and(|output| {
            input.to_token_stream().to_string() == output.to_token_stream().to_string()
        })
    });
    let both_definition = converted_definition(
        both_type,
        quote!(::pons_core::ir::Direction::Both),
        &base_definition,
    );
    let conversion_warning = if (input_type.is_some() || output_type.is_some())
        && both_type.is_none()
    {
        quote!(cx.push_diagnostic("PONS-W101", "serde from/into conversion has different directional shapes; derive input and output separately");)
    } else {
        TokenStream::new()
    };

    quote! {
        impl #impl_generics ::pons_core::Schema for #ident #type_generics #where_clause {
            fn schema(
                cx: &mut ::pons_core::TypeCx,
                direction: ::pons_core::ir::Direction,
            ) -> ::pons_core::ir::TypeRef {
                #conversion_warning
                cx.named::<Self>(#name, direction, #doc, |cx| {
                    match direction {
                        ::pons_core::ir::Direction::Input => #input_definition,
                        ::pons_core::ir::Direction::Output => #output_definition,
                        ::pons_core::ir::Direction::Both => #both_definition,
                    }
                })
            }
        }
    }
}

fn converted_definition(
    ty: Option<&Type>,
    direction: TokenStream,
    base: &TokenStream,
) -> TokenStream {
    match ty {
        Some(ty) => quote!({
            ::pons_core::ir::TypeKind::Newtype {
                inner: <#ty as ::pons_core::Schema>::schema(cx, #direction),
            }
        }),
        None => quote!({ #base }),
    }
}

fn struct_definition(
    style: ast::Style,
    fields: &[ast::Field<'_>],
    attrs: &attr::Container,
) -> TokenStream {
    let mut refusals = Vec::new();
    if let attr::TagType::Internal { tag } = attrs.tag() {
        refusals.push(refuse(format!(
            "serde tag \"{tag}\" is written on every value of this struct, but InlineType has no literal type, so the tag field cannot be described"
        )));
    }
    let shape = if attrs.transparent() {
        transparent_shape(fields, &mut refusals)
    } else if matches!(style, ast::Style::Newtype) {
        newtype_shape(&fields[0])
    } else if matches!(style, ast::Style::Tuple) {
        if !attrs.default().is_none() || fields.iter().any(|f| has_default(&f.attrs)) {
            refusals.push(refuse(
                "serde default fills missing items of a tuple struct, but InlineType::Tuple has no per-item presence".to_owned(),
            ));
        }
        let items = fields.iter().map(field_type_expression);
        quote!(::pons_core::ir::TypeKind::Newtype {
            inner: ::pons_core::ir::TypeRef::Inline(::pons_core::ir::InlineType::Tuple {
                items: vec![#(#items),*],
            }),
        })
    } else {
        named_struct_shape(
            fields,
            attrs.deny_unknown_fields(),
            !attrs.default().is_none(),
        )
    };
    quote!({ #( #refusals )* #shape })
}

fn named_struct_shape(
    fields: &[ast::Field<'_>],
    deny_unknown_fields: bool,
    container_default: bool,
) -> TokenStream {
    let field_builders = fields
        .iter()
        .map(|field| field_expression(field, true, container_default));
    quote! {
        let mut fields = Vec::new();
        let mut flattened = Vec::new();
        #(#field_builders)*
        let openness = if #deny_unknown_fields {
            ::pons_core::ir::ObjectOpenness::Reject
        } else {
            ::pons_core::ir::ObjectOpenness::PreserveOpaque
        };
        ::pons_core::ir::TypeKind::Struct { fields, flattened, openness }
    }
}

fn transparent_shape(fields: &[ast::Field<'_>], refusals: &mut Vec<TokenStream>) -> TokenStream {
    let marked: Vec<&ast::Field<'_>> = fields
        .iter()
        .filter(|field| field.attrs.transparent())
        .collect();
    let [field] = marked.as_slice() else {
        refusals.push(refuse(format!(
            "serde #[serde(transparent)] selects no single field in the derive context pons reads (marked: {}), so the transparent field is ambiguous",
            names(marked.iter().map(|field| &field.member))
        )));
        return quote!(::pons_core::ir::TypeKind::Newtype {
            inner: ::pons_core::ir::TypeRef::Inline(::pons_core::ir::InlineType::Any),
        });
    };
    if field.attrs.skip_deserializing() || has_default(&field.attrs) {
        refusals.push(refuse(format!(
            "serde #[serde(transparent)] marks {} for Serialize, but the Deserialize derive picks another field, and TypeKind::Newtype has one inner slot",
            field.member.to_token_stream()
        )));
    }
    newtype_shape(field)
}

fn newtype_shape(field: &ast::Field<'_>) -> TokenStream {
    let ty = field_type_expression(field);
    quote!(::pons_core::ir::TypeKind::Newtype {
        inner: #ty,
    })
}

fn refuse(message: String) -> TokenStream {
    quote!(cx.push_error("PONS-E101", #message);)
}

fn names<T: ToTokens>(members: impl Iterator<Item = T>) -> String {
    members
        .map(|member| member.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn enum_definition(variants: &[ast::Variant<'_>], attrs: &attr::Container) -> TokenStream {
    let refusals = enum_refusals(variants, attrs);
    let variant_builders = variants.iter().map(|variant| {
        let payload = payload_expression(variant.style, &variant.fields);
        let wire_in = variant.attrs.name().deserialize_name();
        let wire_out = variant.attrs.name().serialize_name();
        let ident = words(&variant.ident.to_string());
        let doc = doc_value(&variant.original.attrs);
        let canonical_name = variant.attrs.name().deserialize_name().to_string();
        let aliases: Vec<_> = variant
            .attrs
            .aliases()
            .iter()
            .filter(|alias| alias.to_string() != canonical_name)
            .map(ToString::to_string)
            .collect();
        let alias_warning = if aliases.is_empty() {
            TokenStream::new()
        } else {
            let message = format!(
                "serde enum aliases {} are not represented separately",
                aliases.join(", ")
            );
            quote!(cx.push_diagnostic("PONS-W101", #message);)
        };
        let other_warning = if variant.attrs.other() {
            quote!(cx.push_diagnostic("PONS-W101", "serde other variant accepts unlisted values");)
        } else {
            TokenStream::new()
        };
        let include_input = !variant.attrs.skip_deserializing();
        let include_output = !variant.attrs.skip_serializing();
        quote! {
            let included = match direction {
                ::pons_core::ir::Direction::Input => #include_input,
                ::pons_core::ir::Direction::Output => #include_output,
                ::pons_core::ir::Direction::Both => #include_input && #include_output,
            };
            if included {
                #alias_warning
                #other_warning
                let wire = match direction {
                    ::pons_core::ir::Direction::Input => #wire_in,
                    ::pons_core::ir::Direction::Output | ::pons_core::ir::Direction::Both => #wire_out,
                };
                variants.push(::pons_core::ir::Variant {
                    wire: wire.to_owned(),
                    ident: #ident,
                    payload: #payload,
                    doc: #doc,
                    diagnostics: Vec::new(),
                    ext: Default::default(),
                });
            }
        }
    });
    let representation = enum_representation(attrs.tag(), variants);
    quote!({
        #( #refusals )*
        let mut variants = Vec::new();
        #(#variant_builders)*
        ::pons_core::ir::TypeKind::Enum { repr: #representation, variants }
    })
}

fn enum_refusals(variants: &[ast::Variant<'_>], attrs: &attr::Container) -> Vec<TokenStream> {
    let mut refusals = Vec::new();
    if attrs.deny_unknown_fields()
        && variants
            .iter()
            .any(|variant| matches!(variant.style, ast::Style::Struct))
    {
        refusals.push(refuse(
            "serde #[serde(deny_unknown_fields)] on an enum rejects unknown keys inside struct variant payloads, but VariantPayload::Struct has no openness field".to_owned(),
        ));
    }
    match attrs.tag() {
        attr::TagType::None => {
            let payloads: Vec<&ast::Variant<'_>> = variants
                .iter()
                .filter(|variant| !matches!(variant.style, ast::Style::Unit))
                .collect();
            if payloads.len() > 1 {
                refusals.push(refuse(format!(
                    "serde untagged enum writes no discriminator, and variants {} carry payloads, so EnumRepr::Untagged cannot tell them apart",
                    names(payloads.iter().map(|variant| &variant.ident))
                )));
            }
        }
        _ => {
            for variant in variants.iter().filter(|v| v.attrs.untagged()) {
                refusals.push(refuse(format!(
                    "serde #[serde(untagged)] variant {} writes no tag, but EnumRepr is per enum, so a mixed tagged and untagged enum cannot be described",
                    variant.ident
                )));
            }
        }
    }
    refusals
}

fn enum_representation(tag: &attr::TagType, variants: &[ast::Variant<'_>]) -> TokenStream {
    match tag {
        attr::TagType::Internal { tag } => {
            quote!(::pons_core::ir::EnumRepr::Internal { tag: #tag.to_owned() })
        }
        attr::TagType::Adjacent { tag, content } => quote!(::pons_core::ir::EnumRepr::Adjacent {
            tag: #tag.to_owned(), content: #content.to_owned(),
        }),
        attr::TagType::None => quote!(::pons_core::ir::EnumRepr::Untagged),
        attr::TagType::External => {
            let input_is_string = variants
                .iter()
                .filter(|variant| !variant.attrs.skip_deserializing())
                .all(|variant| matches!(variant.style, ast::Style::Unit));
            let output_is_string = variants
                .iter()
                .filter(|variant| !variant.attrs.skip_serializing())
                .all(|variant| matches!(variant.style, ast::Style::Unit));
            quote!({
                let string_repr = match direction {
                    ::pons_core::ir::Direction::Input => #input_is_string,
                    ::pons_core::ir::Direction::Output => #output_is_string,
                    ::pons_core::ir::Direction::Both => #input_is_string && #output_is_string,
                };
                if string_repr {
                    ::pons_core::ir::EnumRepr::String
                } else {
                    ::pons_core::ir::EnumRepr::External
                }
            })
        }
    }
}

fn payload_expression(style: ast::Style, fields: &[ast::Field<'_>]) -> TokenStream {
    match style {
        ast::Style::Unit => quote!(::pons_core::ir::VariantPayload::Unit),
        ast::Style::Newtype => {
            let field = fields.first().expect("newtype variants have a field");
            let ty = field_type_expression(field);
            quote!(::pons_core::ir::VariantPayload::Newtype {
                ty: #ty,
            })
        }
        ast::Style::Tuple => {
            let items = fields.iter().map(|field| field_type_expression(field));
            quote!(::pons_core::ir::VariantPayload::Tuple {
                items: vec![#(#items),*]
            })
        }
        ast::Style::Struct => {
            let field_builders = fields
                .iter()
                .map(|field| field_expression(field, false, false));
            quote!({
                let mut fields = Vec::new();
                let mut flattened = Vec::new();
                #(#field_builders)*
                ::pons_core::ir::VariantPayload::Struct { fields }
            })
        }
    }
}

fn field_expression(
    field: &ast::Field<'_>,
    supports_flatten: bool,
    container_default: bool,
) -> TokenStream {
    let skip_input = field.attrs.skip_deserializing();
    let skip_output = field.attrs.skip_serializing();
    let flatten = field.attrs.flatten();
    let codecs = Codecs::new(&field.attrs);
    let canonical_name = field.attrs.name().deserialize_name().to_string();
    let aliases: Vec<_> = field
        .attrs
        .aliases()
        .iter()
        .filter(|alias| alias.to_string() != canonical_name)
        .map(ToString::to_string)
        .collect();
    let alias_warning = if aliases.is_empty() {
        TokenStream::new()
    } else {
        let message = format!(
            "serde aliases {} are not represented as separate fields",
            aliases.join(", ")
        );
        quote!(cx.push_diagnostic("PONS-W101", #message);)
    };
    let included = quote! {
        let included = match direction {
            ::pons_core::ir::Direction::Input => !#skip_input,
            ::pons_core::ir::Direction::Output => !#skip_output,
            ::pons_core::ir::Direction::Both => !#skip_input && !#skip_output,
        };
    };
    let regular = field_body(field, supports_flatten, container_default, flatten, false);
    if !flatten || !codecs.any() {
        return quote! {
            #included
            if included {
                #alias_warning
                #regular
            }
        };
    }
    let opaque = field_body(field, supports_flatten, container_default, flatten, true);
    let probe = codecs.opaque_probe();
    quote! {
        #included
        if included {
            #alias_warning
            if #probe {
                #opaque
            } else {
                #regular
            }
        }
    }
}

fn field_body(
    field: &ast::Field<'_>,
    supports_flatten: bool,
    container_default: bool,
    flatten: bool,
    opaque: bool,
) -> TokenStream {
    let wire_in = field.attrs.name().deserialize_name();
    let wire_out = field.attrs.name().serialize_name();
    let ident = words(&field.member.to_token_stream().to_string());

    let optional_on_input = container_default || has_default(&field.attrs) || is_option(field.ty);
    let optional_on_output = field.attrs.skip_serializing_if().is_some();
    let doc = doc_value(&field.original.attrs);
    let presence = quote! {
        let presence = match direction {
            ::pons_core::ir::Direction::Input => if #optional_on_input { ::pons_core::ir::Presence::Optional } else { ::pons_core::ir::Presence::Required },
            ::pons_core::ir::Direction::Output | ::pons_core::ir::Direction::Both => if #optional_on_output { ::pons_core::ir::Presence::Optional } else { ::pons_core::ir::Presence::Required },
        };
    };
    let ty = field_type_expression(field);
    if opaque {
        return quote! {
            let ty = #ty;
            if #supports_flatten {
                flattened.push(ty);
            } else {
                let wire = match direction {
                    ::pons_core::ir::Direction::Input => #wire_in,
                    ::pons_core::ir::Direction::Output | ::pons_core::ir::Direction::Both => #wire_out,
                };
                #presence
                fields.push(::pons_core::ir::Field {
                    wire: wire.to_owned(),
                    ident: #ident,
                    ty,
                    presence,
                    default: Default::default(),
                    access: None,
                    doc: #doc,
                    diagnostics: vec![::pons_core::ir::Diagnostic {
                        severity: ::pons_core::ir::DiagnosticSeverity::Error,
                        code: "PONS-E101".into(),
                        message: "custom serde flatten in an enum variant payload has no wire key; VariantPayload::Struct cannot represent flattened output".into(),
                        origin: None,
                        ext: Default::default(),
                    }],
                    ext: Default::default(),
                });
            }
        };
    }
    quote! {
        let wire = match direction {
            ::pons_core::ir::Direction::Input => #wire_in,
            ::pons_core::ir::Direction::Output | ::pons_core::ir::Direction::Both => #wire_out,
        };
        let ty = #ty;
        let ty = if #flatten
            && #supports_flatten
            && matches!(
                &ty,
                ::pons_core::ir::TypeRef::Inline(
                    ::pons_core::ir::InlineType::Nullable { .. }
                )
            )
        {
            cx.push_error(
                "PONS-E101",
                "serde flatten of a nullable type is not representable; None omits flattened fields instead of producing null",
            );
            ::pons_core::ir::TypeRef::Inline(::pons_core::ir::InlineType::Any)
        } else {
            ty
        };
        if #flatten {
            if let Some((nested_fields, nested_flattened)) = cx.flattened_fields(&ty)
                && (#supports_flatten || nested_flattened.is_empty())
            {
                fields.extend(nested_fields);
                flattened.extend(nested_flattened);
            } else if #supports_flatten {
                flattened.push(ty);
            } else {
                cx.push_error("PONS-E101", "serde flatten in an enum variant payload is not representable; VariantPayload::Struct has no flattened list, so flattened maps and nested flattened types would be dropped");
            }
        } else {
            #presence
            fields.push(::pons_core::ir::Field {
                wire: wire.to_owned(),
                ident: #ident,
                ty,
                presence,
                default: Default::default(),
                access: None,
                doc: #doc,
                diagnostics: Vec::new(),
                ext: Default::default(),
            });
        }
    }
}

fn field_type_expression(field: &ast::Field<'_>) -> TokenStream {
    let ty = field.ty;
    let codecs = Codecs::new(&field.attrs);
    if !codecs.any() {
        return quote!(<#ty as ::pons_core::Schema>::schema(cx, direction));
    }
    let message = if field.attrs.flatten() {
        "custom serde handler with flatten is opaque; its wire fields are not representable"
    } else {
        "custom serde handler is opaque"
    };
    let input = codecs.label_input;
    let output = codecs.label_output;
    let both = codecs.label_both;
    quote! {{
        let codec = match direction {
            ::pons_core::ir::Direction::Input => #input,
            ::pons_core::ir::Direction::Output => #output,
            ::pons_core::ir::Direction::Both => #both,
        };
        match codec {
            ::std::option::Option::Some(codec) => {
                cx.push_diagnostic("PONS-W101", #message);
                ::pons_core::ir::TypeRef::Inline(::pons_core::ir::InlineType::Opaque {
                    codec,
                    wire: ::pons_core::ir::OpaqueWire::Unknown,
                })
            }
            ::std::option::Option::None => <#ty as ::pons_core::Schema>::schema(cx, direction),
        }
    }}
}

struct Codecs {
    label_input: TokenStream,
    label_output: TokenStream,
    label_both: TokenStream,
    has_codec: bool,
}

impl Codecs {
    fn new(attrs: &attr::Field) -> Self {
        let serialize = attrs
            .serialize_with()
            .map(|path| format!("serialize_with={}", path.to_token_stream()));
        let deserialize = attrs
            .deserialize_with()
            .map(|path| format!("deserialize_with={}", path.to_token_stream()));
        Self {
            label_input: label(deserialize.clone()),
            label_output: label(serialize.clone()),
            label_both: label(match (serialize, deserialize) {
                (Some(serialize), Some(deserialize)) => Some(format!("{serialize},{deserialize}")),
                (serialize, deserialize) => serialize.or(deserialize),
            }),
            has_codec: attrs.serialize_with().is_some() || attrs.deserialize_with().is_some(),
        }
    }

    fn any(&self) -> bool {
        self.has_codec
    }

    fn opaque_probe(&self) -> TokenStream {
        let input = &self.label_input;
        let output = &self.label_output;
        let both = &self.label_both;
        quote! {
            match direction {
                ::pons_core::ir::Direction::Input => #input.is_some(),
                ::pons_core::ir::Direction::Output => #output.is_some(),
                ::pons_core::ir::Direction::Both => #both.is_some(),
            }
        }
    }
}

fn label(codec: Option<String>) -> TokenStream {
    match codec {
        Some(codec) => quote!(::std::option::Option::Some(#codec.to_owned())),
        None => quote!(::std::option::Option::<String>::None),
    }
}

fn has_default(attrs: &attr::Field) -> bool {
    !attrs.default().is_none()
}

fn is_option(ty: &Type) -> bool {
    let Type::Path(path) = ty else { return false };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    segment.ident == "Option"
        && arguments.args.len() == 1
        && matches!(arguments.args.first(), Some(GenericArgument::Type(_)))
}

pub(super) fn doc_value(attrs: &[syn::Attribute]) -> TokenStream {
    let mut lines = Vec::new();
    let mut deprecated = false;
    for attribute in attrs {
        if attribute.path().is_ident("deprecated") {
            deprecated = true;
        }
        let syn::Meta::NameValue(value) = &attribute.meta else {
            continue;
        };
        if !attribute.path().is_ident("doc") {
            continue;
        }
        let syn::Expr::Lit(expression) = &value.value else {
            continue;
        };
        let syn::Lit::Str(line) = &expression.lit else {
            continue;
        };
        lines.push(line.value().trim().to_owned());
    }
    if lines.is_empty() && !deprecated {
        return quote!(None);
    }
    let description = lines.join("\n");
    let summary = lines.iter().find(|line| !line.is_empty()).cloned();
    let summary = match summary {
        Some(summary) => quote!(Some(#summary.to_owned())),
        None => quote!(None),
    };
    quote!(Some(::pons_core::ir::Doc {
        summary: #summary,
        description: if #description.is_empty() { None } else { Some(#description.to_owned()) },
        deprecated: if #deprecated { Some(true) } else { None },
        tags: Vec::new(),
    }))
}

pub(super) fn words(source: &str) -> TokenStream {
    let mut result = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = source.trim_start_matches("r#").chars().collect();
    for (index, character) in chars.iter().copied().enumerate() {
        if character == '_' || character == ' ' {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
        } else {
            if character.is_uppercase() && !current.is_empty() {
                let previous = chars
                    .get(index.wrapping_sub(1))
                    .copied()
                    .unwrap_or_default();
                let next = chars.get(index + 1).copied().unwrap_or_default();
                if previous.is_lowercase() || next.is_lowercase() && previous.is_uppercase() {
                    result.push(std::mem::take(&mut current));
                }
            }
            current.extend(character.to_lowercase());
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    let words = result
        .iter()
        .map(|word| syn::LitStr::new(word, Span::call_site()));
    quote!(vec![#(#words.to_owned()),*])
}
