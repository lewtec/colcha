use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Field, Fields, GenericArgument, Lit, LitBool, LitInt, LitStr,
    PathArguments, Type, parse_macro_input,
};

#[proc_macro_derive(Schema, attributes(colcha))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_schema(&input)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

#[proc_macro_derive(Materialize, attributes(colcha))]
pub fn derive_materialize(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_materialize(&input)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

#[derive(Default)]
struct FieldAttrs {
    key: bool,
    atomic: bool,
    flat: bool,
    native: bool,
    default: Option<Lit>,
    rename: Option<String>,
}

fn field_attrs(field: &Field) -> syn::Result<FieldAttrs> {
    let mut attrs = FieldAttrs::default();
    for attribute in &field.attrs {
        if !attribute.path().is_ident("colcha") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                attrs.key = true;
                Ok(())
            } else if meta.path.is_ident("atomic") {
                attrs.atomic = true;
                Ok(())
            } else if meta.path.is_ident("flat") {
                attrs.flat = true;
                Ok(())
            } else if meta.path.is_ident("native") {
                attrs.native = true;
                Ok(())
            } else if meta.path.is_ident("default") {
                let value = meta.value()?;
                attrs.default = Some(value.parse::<Lit>()?);
                Ok(())
            } else if meta.path.is_ident("rename") {
                let value = meta.value()?;
                let literal = value.parse::<LitStr>()?;
                attrs.rename = Some(literal.value());
                Ok(())
            } else {
                Err(meta.error("unknown colcha attribute"))
            }
        })?;
    }
    Ok(attrs)
}

fn struct_version(attributes: &[Attribute]) -> syn::Result<u32> {
    let mut version = 1u32;
    for attribute in attributes {
        if !attribute.path().is_ident("colcha") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("version") {
                let value = meta.value()?;
                let literal = value.parse::<LitInt>()?;
                version = literal.base10_parse()?;
                Ok(())
            } else {
                Err(meta.error("unknown colcha struct attribute"))
            }
        })?;
    }
    Ok(version)
}

struct EnumAttrs {
    version: u32,
    tag: Option<String>,
}

fn enum_attrs(attributes: &[Attribute]) -> syn::Result<EnumAttrs> {
    let mut attrs = EnumAttrs {
        version: 1,
        tag: None,
    };
    for attribute in attributes {
        if !attribute.path().is_ident("colcha") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("version") {
                let value = meta.value()?;
                let literal = value.parse::<LitInt>()?;
                attrs.version = literal.base10_parse()?;
                Ok(())
            } else if meta.path.is_ident("tag") {
                let value = meta.value()?;
                let literal = value.parse::<LitStr>()?;
                attrs.tag = Some(literal.value());
                Ok(())
            } else {
                Err(meta.error("unknown colcha enum attribute"))
            }
        })?;
    }
    Ok(attrs)
}

fn field_plan(field: &Field) -> syn::Result<(syn::Ident, String, FieldAttrs)> {
    let ident = field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new_spanned(field, "a field needs a name"))?;
    let attrs = field_attrs(field)?;
    if attrs.native && attrs.key {
        return Err(syn::Error::new_spanned(
            field,
            "a native field cannot be a key",
        ));
    }
    if attrs.key && outer_type_name(&field.ty).as_deref() != Some("String") {
        return Err(syn::Error::new_spanned(
            field,
            "#[colcha(key)] needs a String field",
        ));
    }
    if attrs.atomic && !is_vec(&field.ty) && vec_element_inside_option(&field.ty).is_none() {
        return Err(syn::Error::new_spanned(
            field,
            "#[colcha(atomic)] applies to Vec<T>",
        ));
    }
    let segment = attrs
        .rename
        .clone()
        .unwrap_or_else(|| ident.to_string().to_ascii_lowercase());
    Ok((ident, segment, attrs))
}

fn expand_schema(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "colcha derives do not support generic types",
        ));
    }
    match &input.data {
        Data::Struct(data) => schema_struct(input, data),
        Data::Enum(data) => schema_enum(input, data),
        Data::Union(_) => Err(syn::Error::new_spanned(
            input,
            "colcha derives do not support unions",
        )),
    }
}

fn schema_struct(input: &DeriveInput, data: &syn::DataStruct) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let version = struct_version(&input.attrs);
    let version = version?;
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "a Schema struct needs named fields",
        ));
    };

    let mut entries = Vec::new();
    let mut default_entries = Vec::new();
    let mut assertions = Vec::new();
    let mut key_fields = Vec::new();
    let mut has_native = false;

    for field in &fields.named {
        let (ident, segment, attrs) = field_plan(field)?;
        if attrs.key {
            key_fields.push(ident.to_string());
        }
        let node = field_node(field, &attrs)?;
        entries.push(quote! {
            fields.insert(#segment.to_string(), #node);
        });

        let ty = &field.ty;
        let default_entry = if attrs.native {
            has_native = true;
            quote!(return ::std::option::Option::None;)
        } else if let Some(default) = &attrs.default {
            let value = lit_to_value(default)?;
            quote! {
                fields.insert(#segment.to_string(), #value);
            }
        } else {
            quote! {
                match <#ty as ::colcha::NodeOf>::default_value() {
                    ::std::option::Option::Some(value) => {
                        fields.insert(#segment.to_string(), value);
                    }
                    ::std::option::Option::None => return ::std::option::Option::None,
                }
            }
        };
        default_entries.push(default_entry);

        if !attrs.atomic {
            if let Some(element) = vec_element_inside_option(&field.ty) {
                assertions.push(quote! {
                    const _: () = {
                        assert!(
                            <#element as ::colcha::NodeOf>::KEY_FIELD.is_some(),
                            "Vec<T> needs #[colcha(atomic)] or a #[colcha(key)] field on T"
                        );
                    };
                });
            }
        }
    }

    if key_fields.len() > 1 {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "a struct has at most one #[colcha(key)] field",
        ));
    }
    let key_field = if let Some(key) = key_fields.first() {
        quote!(Some(#key))
    } else {
        quote!(None)
    };
    let node_key_field = if let Some(key) = key_fields.first() {
        quote!(Some(#key.to_string()))
    } else {
        quote!(None)
    };
    let default_body = if has_native {
        quote!(::std::option::Option::None)
    } else {
        quote!({
            let mut fields = ::std::collections::BTreeMap::new();
            #(#default_entries)*
            ::std::option::Option::Some(::colcha::Value::Map(fields))
        })
    };

    Ok(quote! {
        #(#assertions)*

        impl ::colcha::NodeOf for #name {
            const KEY_FIELD: Option<&'static str> = #key_field;

            fn default_value() -> ::std::option::Option<::colcha::Value> {
                #default_body
            }

            fn node() -> ::colcha::Node {
                let mut fields = ::std::collections::BTreeMap::new();
                #(#entries)*
                let mut node = ::colcha::Node::structure();
                node.fields = fields;
                node.key_field = #node_key_field;
                node.default = <Self as ::colcha::NodeOf>::default_value();
                node
            }
        }

        impl ::colcha::Schema for #name {
            const NAME: &'static str =
                concat!(module_path!(), "::", stringify!(#name));
            const VERSION: u32 = #version;

            fn schema() -> ::colcha::Node {
                <Self as ::colcha::NodeOf>::node()
            }
        }
    })
}

fn schema_enum(input: &DeriveInput, data: &syn::DataEnum) -> syn::Result<TokenStream2> {
    let attrs = enum_attrs(&input.attrs)?;
    if let Some(tag) = attrs.tag {
        return schema_tagged_enum(input, data, &tag, attrs.version);
    }
    schema_unit_enum(input, data, attrs.version)
}

fn schema_unit_enum(
    input: &DeriveInput,
    data: &syn::DataEnum,
    version: u32,
) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let mut variants = Vec::new();
    let mut default_segment = None;
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "a colcha enum has unit variants only",
            ));
        }
        let attrs = variant_attrs(variant)?;
        let segment = attrs
            .rename
            .unwrap_or_else(|| variant.ident.to_string().to_ascii_lowercase());
        let is_default = attrs.default
            || variant
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("default"));
        if is_default {
            if default_segment.is_some() {
                return Err(syn::Error::new_spanned(
                    variant,
                    "a colcha enum has at most one default variant",
                ));
            }
            default_segment = Some(segment.clone());
        }
        variants.push((variant.ident.clone(), segment));
    }
    let values = variants
        .iter()
        .map(|(_, segment)| quote!(#segment.to_string()));
    let key_field = quote!(None);
    let default_value = match default_segment {
        Some(segment) => quote!(::std::option::Option::Some(::colcha::Value::String(
            #segment.to_string()
        ))),
        None => quote!(::std::option::Option::None),
    };

    Ok(quote! {
        impl ::colcha::NodeOf for #name {
            const KEY_FIELD: Option<&'static str> = #key_field;

            fn default_value() -> ::std::option::Option<::colcha::Value> {
                #default_value
            }

            fn node() -> ::colcha::Node {
                let mut node = ::colcha::Node::leaf(::colcha::FoldPolicy::Flat);
                node.constraints.push(::colcha::Constraint::Type("string".to_string()));
                node.constraints.push(::colcha::Constraint::OneOf(vec![
                    #(::colcha::Value::String(#values)),*
                ]));
                node.default = <Self as ::colcha::NodeOf>::default_value();
                node
            }
        }

        impl ::colcha::Schema for #name {
            const NAME: &'static str =
                concat!(module_path!(), "::", stringify!(#name));
            const VERSION: u32 = #version;

            fn schema() -> ::colcha::Node {
                <Self as ::colcha::NodeOf>::node()
            }
        }
    })
}

fn schema_tagged_enum(
    input: &DeriveInput,
    data: &syn::DataEnum,
    tag: &str,
    version: u32,
) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let mut variant_blocks = Vec::new();
    let mut assertions = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut default_segment: Option<String> = None;
    let mut default_entries: Option<Vec<TokenStream2>> = None;

    for variant in &data.variants {
        let attrs = variant_attrs(variant)?;
        let segment = attrs
            .rename
            .unwrap_or_else(|| variant.ident.to_string().to_ascii_lowercase());
        if !seen.insert(segment.clone()) {
            return Err(syn::Error::new_spanned(
                variant,
                format!("duplicate variant segment '{segment}'"),
            ));
        }
        let is_default = attrs.default
            || variant
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("default"));
        if is_default {
            if default_segment.is_some() {
                return Err(syn::Error::new_spanned(
                    variant,
                    "a colcha enum has at most one default variant",
                ));
            }
            default_segment = Some(segment.clone());
        }

        let block = match &variant.fields {
            Fields::Unit => {
                if is_default {
                    default_entries = Some(Vec::new());
                }
                quote! {
                    {
                        let mut variant = ::colcha::Node::structure();
                        variant.default = ::std::option::Option::Some(
                            ::colcha::Value::Map(::std::collections::BTreeMap::new())
                        );
                        node.variants.insert(#segment.to_string(), variant);
                    }
                }
            }
            Fields::Named(fields) => {
                let (entries, defaults, field_assertions) = variant_fields_tokens(fields, tag)?;
                assertions.extend(field_assertions);
                if is_default {
                    default_entries = Some(defaults.clone());
                }
                quote! {
                    {
                        let mut fields = ::std::collections::BTreeMap::new();
                        #(#entries)*
                        let mut variant = ::colcha::Node::structure();
                        variant.fields = fields;
                        variant.default = (|| -> ::std::option::Option<::colcha::Value> {
                            let mut fields = ::std::collections::BTreeMap::new();
                            #(#defaults)*
                            ::std::option::Option::Some(::colcha::Value::Map(fields))
                        })();
                        node.variants.insert(#segment.to_string(), variant);
                    }
                }
            }
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "a tagged union variant has named fields or no fields",
                ));
            }
        };
        variant_blocks.push(block);
    }

    let default_value = match (default_segment, default_entries) {
        (Some(segment), Some(entries)) => quote! {
            (|| -> ::std::option::Option<::colcha::Value> {
                let mut fields = ::std::collections::BTreeMap::new();
                fields.insert(
                    #tag.to_string(),
                    ::colcha::Value::String(#segment.to_string()),
                );
                #(#entries)*
                ::std::option::Option::Some(::colcha::Value::Map(fields))
            })()
        },
        _ => quote!(::std::option::Option::None),
    };

    Ok(quote! {
        #(#assertions)*

        impl ::colcha::NodeOf for #name {
            const KEY_FIELD: Option<&'static str> = None;

            fn default_value() -> ::std::option::Option<::colcha::Value> {
                #default_value
            }

            fn node() -> ::colcha::Node {
                let mut node = ::colcha::Node::union(#tag);
                #(#variant_blocks)*
                node.default = <Self as ::colcha::NodeOf>::default_value();
                node
            }
        }

        impl ::colcha::Schema for #name {
            const NAME: &'static str =
                concat!(module_path!(), "::", stringify!(#name));
            const VERSION: u32 = #version;

            fn schema() -> ::colcha::Node {
                <Self as ::colcha::NodeOf>::node()
            }
        }
    })
}

fn variant_fields_tokens(
    fields: &syn::FieldsNamed,
    tag: &str,
) -> syn::Result<(Vec<TokenStream2>, Vec<TokenStream2>, Vec<TokenStream2>)> {
    let mut entries = Vec::new();
    let mut default_entries = Vec::new();
    let mut assertions = Vec::new();

    for field in &fields.named {
        let (_, segment, attrs) = field_plan(field)?;
        if attrs.key {
            return Err(syn::Error::new_spanned(
                field,
                "a tagged union variant cannot declare a key field",
            ));
        }
        if segment == tag {
            return Err(syn::Error::new_spanned(
                field,
                format!("the variant field '{segment}' collides with the union tag"),
            ));
        }
        let node = field_node(field, &attrs)?;
        entries.push(quote! {
            fields.insert(#segment.to_string(), #node);
        });

        let ty = &field.ty;
        let default_entry = if attrs.native {
            quote!(return ::std::option::Option::None;)
        } else if let Some(default) = &attrs.default {
            let value = lit_to_value(default)?;
            quote! {
                fields.insert(#segment.to_string(), #value);
            }
        } else {
            quote! {
                match <#ty as ::colcha::NodeOf>::default_value() {
                    ::std::option::Option::Some(value) => {
                        fields.insert(#segment.to_string(), value);
                    }
                    ::std::option::Option::None => return ::std::option::Option::None,
                }
            }
        };
        default_entries.push(default_entry);

        if !attrs.atomic {
            if let Some(element) = vec_element_inside_option(&field.ty) {
                assertions.push(quote! {
                    const _: () = {
                        assert!(
                            <#element as ::colcha::NodeOf>::KEY_FIELD.is_some(),
                            "Vec<T> needs #[colcha(atomic)] or a #[colcha(key)] field on T"
                        );
                    };
                });
            }
        }
    }
    Ok((entries, default_entries, assertions))
}

#[derive(Default)]
struct VariantAttrs {
    rename: Option<String>,
    default: bool,
}

fn variant_attrs(variant: &syn::Variant) -> syn::Result<VariantAttrs> {
    let mut attrs = VariantAttrs::default();
    for attribute in &variant.attrs {
        if !attribute.path().is_ident("colcha") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value = meta.value()?;
                let literal = value.parse::<LitStr>()?;
                attrs.rename = Some(literal.value());
                Ok(())
            } else if meta.path.is_ident("default") {
                attrs.default = true;
                Ok(())
            } else {
                Err(meta.error("unknown colcha variant attribute"))
            }
        })?;
    }
    Ok(attrs)
}

fn field_node(field: &Field, attrs: &FieldAttrs) -> syn::Result<TokenStream2> {
    let ty = &field.ty;
    if attrs.native {
        if is_option(ty) {
            return Err(syn::Error::new_spanned(
                field,
                "an optional native field is not supported",
            ));
        }
        return Ok(quote!({
            let mut node = ::colcha::Node::native(<#ty as ::colcha::NativeType>::NAME);
            node
        }));
    }

    let mut statements = Vec::new();
    if attrs.flat {
        statements.push(quote!(node.policy = ::colcha::FoldPolicy::Flat;));
    }
    if attrs.atomic {
        statements.push(quote!(node.policy = ::colcha::FoldPolicy::Atomic;));
        statements.push(quote!(node.key_field = None;));
        statements.push(quote!(node.kind = ::colcha::NodeKind::List;));
    }
    if let Some(default) = &attrs.default {
        let value = lit_to_value(default)?;
        statements.push(quote!(node.default = Some(#value);));
    }

    Ok(quote!({
        let mut node = <#ty as ::colcha::NodeOf>::node();
        #(#statements)*
        node
    }))
}

fn expand_materialize(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "colcha derives do not support generic types",
        ));
    }
    match &input.data {
        Data::Struct(data) => materialize_struct(input, data),
        Data::Enum(data) => materialize_enum(input, data),
        Data::Union(_) => Err(syn::Error::new_spanned(
            input,
            "colcha derives do not support unions",
        )),
    }
}

fn materialize_struct(input: &DeriveInput, data: &syn::DataStruct) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "a Materialize struct needs named fields",
        ));
    };

    let mut field_exprs = Vec::new();
    for field in &fields.named {
        let (ident, segment, attrs) = field_plan(field)?;
        let value = materialize_value_expr(&field.ty, &attrs)?;
        field_exprs.push(quote! {
            #ident: {
                let field_path = path.child(#segment).map_err(|error| {
                    ::colcha::Error::Materialize {
                        path: path.clone(),
                        message: error.to_string(),
                    }
                })?;
                let field_value = map.get(#segment).unwrap_or(&::colcha::Value::Null);
                #value
            },
        });
    }

    Ok(quote! {
        impl ::colcha::Materialize for #name {
            fn materialize(
                value: &::colcha::Value,
                host: &dyn ::colcha::NativeHost,
                path: &::colcha::Path,
            ) -> ::std::result::Result<Self, ::colcha::Error> {
                let map = match value {
                    ::colcha::Value::Map(map) => map,
                    other => {
                        return Err(::colcha::Error::Materialize {
                            path: path.clone(),
                            message: format!(
                                "expected map, found {}",
                                other.type_name()
                            ),
                        });
                    }
                };
                Ok(Self {
                    #(#field_exprs)*
                })
            }
        }
    })
}

fn materialize_enum(input: &DeriveInput, data: &syn::DataEnum) -> syn::Result<TokenStream2> {
    let attrs = enum_attrs(&input.attrs)?;
    if let Some(tag) = attrs.tag {
        return materialize_tagged_enum(input, data, &tag);
    }
    materialize_unit_enum(input, data)
}

fn materialize_unit_enum(input: &DeriveInput, data: &syn::DataEnum) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let mut arms = Vec::new();
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "a colcha enum has unit variants only",
            ));
        }
        let attrs = variant_attrs(variant)?;
        let segment = attrs
            .rename
            .unwrap_or_else(|| variant.ident.to_string().to_ascii_lowercase());
        let ident = &variant.ident;
        arms.push(quote!(#segment => Ok(Self::#ident),));
    }

    Ok(quote! {
        impl ::colcha::Materialize for #name {
            fn materialize(
                value: &::colcha::Value,
                _host: &dyn ::colcha::NativeHost,
                path: &::colcha::Path,
            ) -> ::std::result::Result<Self, ::colcha::Error> {
                let text = match value {
                    ::colcha::Value::String(text) => text,
                    other => {
                        return Err(::colcha::Error::Materialize {
                            path: path.clone(),
                            message: format!(
                                "expected enum string, found {}",
                                other.type_name()
                            ),
                        });
                    }
                };
                match text.as_str() {
                    #(#arms)*
                    other => Err(::colcha::Error::Materialize {
                        path: path.clone(),
                        message: format!("unknown variant '{other}'"),
                    }),
                }
            }
        }
    })
}

fn materialize_tagged_enum(
    input: &DeriveInput,
    data: &syn::DataEnum,
    tag: &str,
) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let mut arms = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for variant in &data.variants {
        let attrs = variant_attrs(variant)?;
        let segment = attrs
            .rename
            .unwrap_or_else(|| variant.ident.to_string().to_ascii_lowercase());
        if !seen.insert(segment.clone()) {
            return Err(syn::Error::new_spanned(
                variant,
                format!("duplicate variant segment '{segment}'"),
            ));
        }
        let ident = &variant.ident;
        let arm = match &variant.fields {
            Fields::Unit => quote! {
                #segment => {
                    let allowed: &[&str] = &[];
                    for key in map.keys() {
                        if key.as_str() != #tag && !allowed.contains(&key.as_str()) {
                            return Err(::colcha::Error::Materialize {
                                path: path.clone(),
                                message: format!(
                                    "unknown field '{}' in variant '{}'",
                                    key, #segment
                                ),
                            });
                        }
                    }
                    Ok(Self::#ident)
                }
            },
            Fields::Named(fields) => {
                let mut field_inits = Vec::new();
                let mut field_segments = Vec::new();
                for field in &fields.named {
                    let (field_ident, field_segment, field_attrs) = field_plan(field)?;
                    if field_attrs.key {
                        return Err(syn::Error::new_spanned(
                            field,
                            "a tagged union variant cannot declare a key field",
                        ));
                    }
                    if field_segment == tag {
                        return Err(syn::Error::new_spanned(
                            field,
                            format!(
                                "the variant field '{field_segment}' collides with the union tag"
                            ),
                        ));
                    }
                    let value = materialize_value_expr(&field.ty, &field_attrs)?;
                    let fallback = match &field_attrs.default {
                        Some(default) => {
                            let value = lit_to_value(default)?;
                            quote!(::std::option::Option::Some(#value))
                        }
                        None => {
                            let ty = &field.ty;
                            quote!(<#ty as ::colcha::NodeOf>::default_value())
                        }
                    };
                    field_inits.push(quote! {
                        #field_ident: {
                            let field_path = path.child(#field_segment).map_err(|error| {
                                ::colcha::Error::Materialize {
                                    path: path.clone(),
                                    message: error.to_string(),
                                }
                            })?;
                            let fallback_value;
                            let field_value = match map.get(#field_segment) {
                                ::std::option::Option::Some(value) => value,
                                ::std::option::Option::None => {
                                    fallback_value = #fallback
                                        .unwrap_or(::colcha::Value::Null);
                                    &fallback_value
                                }
                            };
                            #value
                        },
                    });
                    field_segments.push(field_segment);
                }
                quote! {
                    #segment => {
                        let allowed: &[&str] = &[#(#field_segments),*];
                        for key in map.keys() {
                            if key.as_str() != #tag && !allowed.contains(&key.as_str()) {
                                return Err(::colcha::Error::Materialize {
                                    path: path.clone(),
                                    message: format!(
                                        "unknown field '{}' in variant '{}'",
                                        key, #segment
                                    ),
                                });
                            }
                        }
                        Ok(Self::#ident {
                            #(#field_inits)*
                        })
                    }
                }
            }
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "a tagged union variant has named fields or no fields",
                ));
            }
        };
        arms.push(arm);
    }

    Ok(quote! {
        impl ::colcha::Materialize for #name {
            fn materialize(
                value: &::colcha::Value,
                host: &dyn ::colcha::NativeHost,
                path: &::colcha::Path,
            ) -> ::std::result::Result<Self, ::colcha::Error> {
                let map = match value {
                    ::colcha::Value::Map(map) => map,
                    other => {
                        return Err(::colcha::Error::Materialize {
                            path: path.clone(),
                            message: format!(
                                "expected tagged union object, found {}",
                                other.type_name()
                            ),
                        });
                    }
                };
                let tag_value = map.get(#tag).ok_or_else(|| {
                    ::colcha::Error::Materialize {
                        path: path.clone(),
                        message: format!("missing tag field '{}'", #tag),
                    }
                })?;
                let variant = match tag_value {
                    ::colcha::Value::String(variant) => variant.as_str(),
                    other => {
                        return Err(::colcha::Error::Materialize {
                            path: path.clone(),
                            message: format!(
                                "tag '{}' is a string, found {}",
                                #tag,
                                other.type_name()
                            ),
                        });
                    }
                };
                match variant {
                    #(#arms)*
                    other => Err(::colcha::Error::Materialize {
                        path: path.clone(),
                        message: format!("unknown variant '{other}'"),
                    }),
                }
            }
        }
    })
}

fn materialize_value_expr(ty: &Type, attrs: &FieldAttrs) -> syn::Result<TokenStream2> {
    if attrs.native {
        Ok(quote! {
            {
                let (native_name, native_args) = match field_value {
                    ::colcha::Value::Native { name, args } => (name.clone(), args.as_ref()),
                    other => {
                        return Err(::colcha::Error::Materialize {
                            path: field_path.clone(),
                            message: format!(
                                "expected native value, found {}",
                                other.type_name()
                            ),
                        });
                    }
                };
                let handle = host.build(&native_name, native_args).map_err(|source| {
                    ::colcha::Error::Build {
                        path: field_path.clone(),
                        name: native_name.clone(),
                        source,
                    }
                })?;
                <#ty as ::colcha::NativeType>::from_native(handle).map_err(|message| {
                    ::colcha::Error::Materialize {
                        path: field_path.clone(),
                        message,
                    }
                })?
            }
        })
    } else {
        Ok(quote! {
            <#ty as ::colcha::Materialize>::materialize(
                field_value,
                host,
                &field_path,
            )?
        })
    }
}

fn lit_to_value(literal: &Lit) -> syn::Result<TokenStream2> {
    match literal {
        Lit::Str(value) => {
            let text = value.value();
            Ok(quote!(::colcha::Value::String(#text.to_string())))
        }
        Lit::Int(value) => {
            let number: i64 = value.base10_parse()?;
            Ok(quote!(::colcha::Value::Int(#number)))
        }
        Lit::Float(value) => {
            let number: f64 = value.base10_parse()?;
            Ok(quote!(
                ::colcha::Value::Float(
                    ::colcha::Float::new(#number).expect("a default cannot be NaN")
                )
            ))
        }
        Lit::Bool(LitBool { value, .. }) => Ok(quote!(::colcha::Value::Bool(#value))),
        Lit::Char(value) => {
            let text = value.value().to_string();
            Ok(quote!(::colcha::Value::String(#text.to_string())))
        }
        _ => Err(syn::Error::new_spanned(
            literal,
            "unsupported colcha default literal",
        )),
    }
}

fn outer_type_name(ty: &Type) -> Option<String> {
    if let Type::Path(path) = ty {
        path.path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
    } else {
        None
    }
}

fn generic_inner(ty: &Type, outer: &str) -> Option<Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != outer {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| {
        if let GenericArgument::Type(ty) = argument {
            Some(ty.clone())
        } else {
            None
        }
    })
}

fn is_vec(ty: &Type) -> bool {
    outer_type_name(ty).as_deref() == Some("Vec")
}

fn is_option(ty: &Type) -> bool {
    outer_type_name(ty).as_deref() == Some("Option")
}

fn vec_element_inside_option(ty: &Type) -> Option<Type> {
    if let Some(inner) = generic_inner(ty, "Vec") {
        return Some(inner);
    }
    if let Some(inner) = generic_inner(ty, "Option") {
        return generic_inner(&inner, "Vec");
    }
    None
}
