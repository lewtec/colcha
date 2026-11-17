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
    let mut assertions = Vec::new();
    let mut key_fields = Vec::new();

    for field in &fields.named {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "a field needs a name"))?;
        let attrs = field_attrs(field)?;
        if attrs.native && attrs.key {
            return Err(syn::Error::new_spanned(
                field,
                "a native field cannot be a key",
            ));
        }
        if attrs.key {
            if outer_type_name(&field.ty).as_deref() != Some("String") {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[colcha(key)] needs a String field",
                ));
            }
            key_fields.push(ident.to_string());
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
        let node = field_node(field, &attrs)?;
        entries.push(quote! {
            fields.insert(#segment.to_string(), #node);
        });

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

    Ok(quote! {
        #(#assertions)*

        impl ::colcha::NodeOf for #name {
            const KEY_FIELD: Option<&'static str> = #key_field;

            fn node() -> ::colcha::Node {
                let mut fields = ::std::collections::BTreeMap::new();
                #(#entries)*
                let mut node = ::colcha::Node::structure();
                node.fields = fields;
                node.key_field = #node_key_field;
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
    let name = &input.ident;
    let version = struct_version(&input.attrs)?;
    let mut variants = Vec::new();
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
        variants.push((variant.ident.clone(), segment));
    }
    let values = variants
        .iter()
        .map(|(_, segment)| quote!(#segment.to_string()));
    let key_field = quote!(None);

    Ok(quote! {
        impl ::colcha::NodeOf for #name {
            const KEY_FIELD: Option<&'static str> = #key_field;

            fn node() -> ::colcha::Node {
                let mut node = ::colcha::Node::leaf(::colcha::FoldPolicy::Flat);
                node.constraints.push(::colcha::Constraint::Type("string".to_string()));
                node.constraints.push(::colcha::Constraint::OneOf(vec![
                    #(::colcha::Value::String(#values)),*
                ]));
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

#[derive(Default)]
struct VariantAttrs {
    rename: Option<String>,
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
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "a field needs a name"))?;
        let attrs = field_attrs(field)?;
        let segment = attrs
            .rename
            .clone()
            .unwrap_or_else(|| ident.to_string().to_ascii_lowercase());
        let ty = &field.ty;
        let value = if attrs.native {
            quote! {
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
            }
        } else {
            quote! {
                <#ty as ::colcha::Materialize>::materialize(
                    field_value,
                    host,
                    &field_path,
                )?
            }
        };
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
