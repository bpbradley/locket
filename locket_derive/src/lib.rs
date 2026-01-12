use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Error, Expr, ExprLit, Field, Fields, Ident, Lit, LitStr, Meta,
    Path, Type, parse_macro_input,
};

struct FieldInfo<'a> {
    field: &'a Field,
    ident: &'a Ident,
    ty: &'a Type,
    is_option: bool,
    is_flattened: bool,
    clap_long_name: String,
    cfgs: Vec<&'a Attribute>,
    locket: LocketFieldAttrs,
}

#[derive(Default)]
struct LocketFieldAttrs {
    skip: bool,
    optional: bool,
    default: Option<Expr>,
    overlay_fn: Option<Path>,
    docs: Option<String>,
    allow_mismatched_flatten: bool,
}

#[derive(Default)]
struct LocketStructAttrs {
    try_into: Option<Path>,
    section: Option<String>,
}

#[proc_macro_derive(LayeredConfig, attributes(locket))]
pub fn derive_layered_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // struct level
    let struct_attrs = match parse_struct_attrs(&input.attrs) {
        Ok(attrs) => attrs,
        Err(e) => return e.into_compile_error().into(),
    };

    // field level
    let struct_info = match parse_field_info(&input) {
        Ok(info) => info,
        Err(e) => return e.into_compile_error().into(),
    };

    let struct_name = &input.ident;

    let overlay_impl = generate_overlay_impl(&struct_info);
    let defaults_impl = generate_defaults_impl(&struct_info);
    let section_impl = generate_section_impl(&struct_attrs);
    let doc_defaults_impl = generate_doc_defaults_impl(&struct_info, struct_name);
    let structure_impl = generate_structure_impl(&struct_info);

    let try_from_impl = match generate_try_from_impl(struct_name, &struct_attrs, &struct_info) {
        Ok(tokens) => tokens,
        Err(e) => return e.into_compile_error().into(),
    };

    let expanded = quote! {
        #[automatically_derived]
        impl crate::config::Overlay for #struct_name {
            fn overlay(self, top: Self) -> Self {
                #overlay_impl
            }
        }

        #[automatically_derived]
        impl crate::config::ApplyDefaults for #struct_name {
            fn apply_defaults(self) -> Self {
                #defaults_impl
            }
        }

        #[automatically_derived]
        impl crate::config::ConfigSection for #struct_name {
            fn section_name() -> Option<&'static str> {
                #section_impl
            }
        }

        #[cfg(feature = "locket-docs")]
        #doc_defaults_impl

        #[cfg(feature = "locket-docs")]
        #[automatically_derived]
        impl crate::config::ConfigStructure for #struct_name {
            fn get_structure() -> Vec<(String, Option<String>)> {
                #structure_impl
            }
        }

        #try_from_impl
    };

    TokenStream::from(expanded)
}

fn parse_struct_attrs(attrs: &[Attribute]) -> syn::Result<LocketStructAttrs> {
    let mut info = LocketStructAttrs::default();

    for attr in attrs {
        if !attr.path().is_ident("locket") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("try_into") {
                let val = meta.value()?;
                let s: LitStr = val.parse()?;
                info.try_into = Some(s.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("section") {
                let val = meta.value()?;
                let s: LitStr = val.parse()?;
                info.section = Some(s.value());
                return Ok(());
            }
            // Ignore unknown keys at struct level
            Ok(())
        })?;
    }
    Ok(info)
}

fn parse_field_info<'a>(input: &'a DeriveInput) -> syn::Result<Vec<FieldInfo<'a>>> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new(
            input.span(),
            "#[derive(LayeredConfig)] only supports structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new(
            input.span(),
            "#[derive(LayeredConfig)] only supports named fields",
        ));
    };

    let mut infos = Vec::new();

    for field in &fields.named {
        let ident = field.ident.as_ref().unwrap();
        let locket = parse_locket_field_attrs(&field.attrs)?;
        let cfgs: Vec<&Attribute> = field
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("cfg"))
            .collect();
        let is_option = is_type_option(&field.ty);
        let clap_long_name = get_clap_long_name(field);

        let has_clap_flatten = ["command", "clap", "arg"]
            .iter()
            .any(|k| has_attribute(&field.attrs, k, "flatten"));
        let has_serde_flatten = has_attribute(&field.attrs, "serde", "flatten");
        let is_flattened = has_clap_flatten || has_serde_flatten;

        if !locket.allow_mismatched_flatten && (has_clap_flatten != has_serde_flatten) {
            return Err(Error::new(
                field.span(),
                "Locket: Mismatched flattening! Ensure both Clap and Serde use `flatten` or use #[locket(allow_mismatched_flatten)].",
            ));
        }

        let has_clap_default = ["clap", "arg"].iter().any(|k| {
            has_attribute(&field.attrs, k, "default_value")
                || has_attribute(&field.attrs, k, "default_value_t")
        });
        if has_clap_default {
            return Err(Error::new(
                field.span(),
                "Locket: Clap default detected! Use `#[locket(default = ...)]` instead.",
            ));
        }

        infos.push(FieldInfo {
            field,
            ident,
            ty: &field.ty,
            is_option,
            is_flattened,
            clap_long_name,
            cfgs,
            locket,
        });
    }
    Ok(infos)
}

fn parse_locket_field_attrs(attrs: &[Attribute]) -> syn::Result<LocketFieldAttrs> {
    let mut info = LocketFieldAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("locket") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                info.skip = true;
            } else if meta.path.is_ident("optional") {
                info.optional = true;
            } else if meta.path.is_ident("default") {
                let val = meta.value()?;
                info.default = Some(val.parse()?);
            } else if meta.path.is_ident("overlay") {
                let val = meta.value()?;
                let s: LitStr = val.parse()?;
                info.overlay_fn = Some(s.parse()?);
            } else if meta.path.is_ident("docs") {
                let val = meta.value()?;
                let s: LitStr = val.parse()?;
                info.docs = Some(s.value());
            } else if meta.path.is_ident("allow_mismatched_flatten") {
                info.allow_mismatched_flatten = true;
            }
            Ok(())
        })?;
    }
    Ok(info)
}

fn generate_overlay_impl(infos: &[FieldInfo]) -> TokenStream2 {
    let assignments = infos.iter().map(|info| {
        let name = info.ident;
        let cfgs = &info.cfgs;
        if let Some(func) = &info.locket.overlay_fn {
            quote_spanned! {info.field.span()=> #(#cfgs)* #name: #func(self.#name, top.#name) }
        } else {
            quote_spanned! {info.field.span()=> #(#cfgs)* #name: self.#name.overlay(top.#name) }
        }
    });
    quote! { Self { #(#assignments),* } }
}

fn generate_defaults_impl(infos: &[FieldInfo]) -> TokenStream2 {
    let assignments = infos.iter().map(|info| {
        let name = info.ident;
        let cfgs = &info.cfgs;

        if info.is_flattened {
            return quote_spanned! {info.field.span()=> #(#cfgs)* #name: self.#name.apply_defaults() };
        }

        if let Some(expr) = &info.locket.default {
            let is_string_lit = matches!(expr, Expr::Lit(ExprLit { lit: Lit::Str(_), .. }));
            if is_string_lit {
                quote_spanned! {info.field.span()=>
                    #(#cfgs)*
                    #name: self.#name.or_else(|| Some(
                        #expr.parse().expect(concat!("Locket: Invalid default value for '", stringify!(#name), "'"))
                    ))
                }
            } else {
                quote_spanned! {info.field.span()=>
                    #(#cfgs)*
                    #[allow(clippy::useless_conversion)]
                    #name: self.#name.or_else(|| Some((#expr).into()))
                }
            }
        } else {
            quote_spanned! {info.field.span()=> #(#cfgs)* #name: self.#name }
        }
    });
    quote! { Self { #(#assignments),* } }
}

fn generate_section_impl(attrs: &LocketStructAttrs) -> TokenStream2 {
    match &attrs.section {
        Some(s) => quote! { Some(#s) },
        None => quote! { None },
    }
}

fn generate_structure_impl(infos: &[FieldInfo]) -> TokenStream2 {
    let recurse = infos.iter().map(|info| {
        let name = info.ident;
        let cfgs = &info.cfgs;

        if info.is_flattened {
            let ty = info.ty;
            quote! { #(#cfgs)* keys.extend(<#ty as crate::config::ConfigStructure>::get_structure()); }
        } else {
            let key = name.to_string().replace('_', "-");
            let docs_code = match &info.locket.docs {
                Some(s) => quote! { Some(#s.to_string()) },
                None => quote! { None },
            };
            quote! { #(#cfgs)* keys.push((#key.to_string(), #docs_code)); }
        }
    });
    quote! {
        let mut keys = Vec::new();
        #(#recurse)*
        keys
    }
}

fn generate_doc_defaults_impl(infos: &[FieldInfo], struct_name: &Ident) -> TokenStream2 {
    let statements = infos.iter().map(|info| {
        if info.locket.skip {
            return quote! {};
        }
        if info.is_flattened {
            let ty = info.ty;
            return quote! { <#ty as crate::config::LocketDocDefaults>::register_defaults(map); };
        }
        if let Some(expr) = &info.locket.default {
            let flag_name = &info.clap_long_name;
            match expr {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => {
                    let val = s.value();
                    quote! { map.insert(#flag_name.to_string(), #val.to_string()); }
                }
                _ => quote! { map.insert(#flag_name.to_string(), (#expr).to_string()); },
            }
        } else {
            quote! {}
        }
    });
    quote! {
        #[automatically_derived]
        impl crate::config::LocketDocDefaults for #struct_name {
            fn register_defaults(map: &mut std::collections::HashMap<String, String>) {
                #(#statements)*
            }
        }
    }
}

fn generate_try_from_impl(
    struct_name: &Ident,
    attrs: &LocketStructAttrs,
    infos: &[FieldInfo],
) -> syn::Result<TokenStream2> {
    let Some(target) = &attrs.try_into else {
        return Ok(quote! {});
    };

    let fields = infos.iter().filter_map(|info| {
        if info.locket.skip { return None; }
        let name = info.ident;
        let cfgs = &info.cfgs;

        // Logic:
        // If source is NOT Option -> It is mandatory. Convert directly.
        // If source is Option:
        //    a. Default? -> UnwrapOr(default) -> Convert.
        //    b. Is Explicitly Optional (Target is Option)? -> Map -> Convert -> Transpose.
        //    c. Is Implicitly Mandatory (Target is T)? -> OkOr(Error) -> Convert.

        let expr = if !info.is_option {
            quote_spanned! {info.field.span()=> args.#name.try_into()? }
        } else if info.locket.default.is_some() {
            quote_spanned! {info.field.span()=>
                args.#name
                    .ok_or_else(|| crate::config::ConfigError::Validation(
                        format!("Missing field '{}' despite default existing. Did you call apply_defaults()?", stringify!(#name)).into()
                    ))?
                    .try_into()?
            }
        } else if info.locket.optional {
            // Target is explicitly Option<U>
            quote_spanned! {info.field.span()=>
                args.#name.map(|v| v.try_into()).transpose()?
            }
        } else {
            // Target is U (Strict)
            let flag_literal = format!("--{}", info.clap_long_name);
            let err_msg = format!("Missing required configuration field: {}", flag_literal);
            quote_spanned! {info.field.span()=>
                args.#name
                    .ok_or_else(|| crate::config::ConfigError::Validation(#err_msg.into()))?
                    .try_into()?
            }
        };

        Some(quote! {
            #(#cfgs)*
            #[allow(clippy::useless_conversion)]
            #name: #expr
        })
    });

    Ok(quote! {
        impl TryFrom<#struct_name> for #target {
            type Error = crate::error::LocketError;

            fn try_from(args: #struct_name) -> Result<Self, Self::Error> {
                #[allow(clippy::unnecessary_fallible_conversions)]
                Ok(Self {
                    #(#fields),*
                })
            }
        }
    })
}

fn is_type_option(ty: &Type) -> bool {
    if let Type::Path(tp) = ty {
        tp.path
            .segments
            .last()
            .map(|s| s.ident == "Option")
            .unwrap_or(false)
    } else {
        false
    }
}

fn has_attribute(attrs: &[Attribute], path_ident: &str, nested_ident: &str) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident(path_ident) {
            return false;
        }
        let mut found = false;
        if let Meta::List(meta) = &attr.meta {
            let _ = meta.parse_nested_meta(|m| {
                if m.path.is_ident(nested_ident) {
                    found = true;
                }
                Ok(())
            });
        }
        found
    })
}

fn get_clap_long_name(field: &Field) -> String {
    let default_name = field.ident.as_ref().unwrap().to_string().replace('_', "-");
    for attr in &field.attrs {
        if !attr.path().is_ident("arg") && !attr.path().is_ident("clap") {
            continue;
        }
        let mut explicit_name = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("long") {
                match meta.value() {
                    Ok(val) => {
                        if let Ok(lit) = val.parse::<LitStr>() {
                            explicit_name = Some(lit.value());
                        }
                    }
                    Err(_) => explicit_name = Some(default_name.clone()),
                }
            }
            Ok(())
        });
        if let Some(name) = explicit_name {
            return name;
        }
    }
    default_name
}
