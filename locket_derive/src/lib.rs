use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Field, Fields, Lit, LitStr, Meta, Type,
    parse_macro_input,
};

#[proc_macro_derive(LayeredConfig, attributes(locket))]
pub fn derive_layered_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // Fail fast if attributes are invalid
    if let Err(e) = validate_attributes(&input.data) {
        return e.into_compile_error().into();
    }

    let struct_name = input.ident;

    // Generate all implementations
    let overlay_logic = generate_overlay_body(&input.data);
    let defaults_logic = generate_defaults_body(&input.data);
    let doc_defaults_logic = generate_doc_defaults_impl(&input.data, &struct_name);

    // Check for #[locket(try_into = "Path::To::Target")]
    let config_target = input.attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("locket") {
            return None;
        }
        match attr.parse_args::<Meta>().ok()? {
            Meta::NameValue(nv) if nv.path.is_ident("try_into") => {
                if let Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) = nv.value
                {
                    s.parse::<syn::Path>().ok()
                } else {
                    None
                }
            }
            _ => None,
        }
    });

    let try_from_impl = if let Some(target) = config_target {
        let mapping_logic = generate_try_from_body(&input.data);
        quote! {
            impl TryFrom<#struct_name> for #target {
                type Error = crate::error::LocketError;

                fn try_from(args: #struct_name) -> Result<Self, Self::Error> {
                    let args = <#struct_name as crate::config::ApplyDefaults>::apply_defaults(args);
                    Ok(Self {
                        #mapping_logic
                    })
                }
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #[automatically_derived]
        impl crate::config::Overlay for #struct_name {
            fn overlay(self, top: Self) -> Self {
                #overlay_logic
            }
        }

        #[automatically_derived]
        impl crate::config::ApplyDefaults for #struct_name {
            fn apply_defaults(self) -> Self {
                #defaults_logic
            }
        }

        #doc_defaults_logic

        #try_from_impl
    };

    TokenStream::from(expanded)
}

fn generate_overlay_body(data: &Data) -> proc_macro2::TokenStream {
    match data {
        Data::Struct(struct_data) => match &struct_data.fields {
            Fields::Named(fields) => {
                let recurse = fields.named.iter().map(|f| {
                    let name = &f.ident;
                    quote_spanned! {f.span()=>
                        #name: self.#name.overlay(top.#name)
                    }
                });
                quote! { Self { #(#recurse),* } }
            }
            Fields::Unnamed(fields) => {
                let recurse = fields.unnamed.iter().enumerate().map(|(i, f)| {
                    let index = syn::Index::from(i);
                    quote_spanned! {f.span()=>
                        self.#index.overlay(top.#index)
                    }
                });
                quote! { Self( #(#recurse),* ) }
            }
            Fields::Unit => quote! { Self },
        },
        _ => panic!("#[derive(LayeredConfig)] only works on structs"),
    }
}

fn generate_defaults_body(data: &Data) -> proc_macro2::TokenStream {
    match data {
        Data::Struct(struct_data) => match &struct_data.fields {
            Fields::Named(fields) => {
                let recurse = fields.named.iter().map(|f| {
                    let name = &f.ident;

                    // Parse attributes for #[locket(default = ...)]
                    let default_expr = f.attrs.iter().find_map(|attr| {
                        if !attr.path().is_ident("locket") { return None; }
                        match attr.parse_args::<Meta>().ok()? {
                            Meta::NameValue(nv) if nv.path.is_ident("default") => Some(nv.value),
                            _ => None,
                        }
                    });

                    if let Some(expr) = default_expr {
                        // Handle String literals specifically for .parse()
                        let is_string_lit = matches!(
                            expr,
                            Expr::Lit(ExprLit { lit: Lit::Str(_), .. })
                        );

                        if is_string_lit {
                            quote_spanned! {f.span()=>
                                #name: self.#name.or_else(|| Some(
                                    #expr.parse().expect(concat!("Locket: Invalid default value for field '", stringify!(#name), "'"))
                                ))
                            }
                        } else {
                            // Handle numbers/bools/consts
                            quote_spanned! {f.span()=>
                                #name: self.#name.or_else(|| {
                                    #[allow(clippy::useless_conversion)]
                                    Some((#expr).into())
                                })
                            }
                        }
                    } else {
                        quote_spanned! {f.span()=> #name: self.#name }
                    }
                });
                quote! { Self { #(#recurse),* } }
            }
            _ => quote! { self },
        },
        _ => quote! { self },
    }
}

fn generate_doc_defaults_impl(data: &Data, struct_name: &syn::Ident) -> proc_macro2::TokenStream {
    let body = match data {
        Data::Struct(struct_data) => match &struct_data.fields {
            Fields::Named(fields) => {
                let statements = fields.named.iter().map(|f| {
                    let is_flattened = ["command", "clap", "arg", "serde"]
                        .iter()
                        .any(|key| has_attribute(&f.attrs, key, "flatten"));

                    if is_flattened {
                        let ty = &f.ty;
                        return quote! {
                            <#ty as crate::config::LocketDocDefaults>::register_defaults(map);
                        };
                    }

                    let is_skipped = f.attrs.iter().any(|a| {
                        a.path().is_ident("locket")
                            && a.parse_args::<Meta>()
                                .is_ok_and(|m| matches!(m, Meta::Path(p) if p.is_ident("skip")))
                    });
                    if is_skipped {
                        return quote! {};
                    }

                    let default_val = f.attrs.iter().find_map(|attr| {
                        if !attr.path().is_ident("locket") {
                            return None;
                        }
                        match attr.parse_args::<Meta>().ok()? {
                            Meta::NameValue(nv) if nv.path.is_ident("default") => Some(nv.value),
                            _ => None,
                        }
                    });

                    if let Some(expr) = default_val {
                        let flag_name = get_clap_long_name(f);

                        // Convert expression to string for documentation
                        let default_str = match expr {
                            Expr::Lit(ExprLit {
                                lit: Lit::Str(s), ..
                            }) => s.value(),
                            // For complex expressions, fallback to stringifying the code tokens
                            _ => quote!(#expr).to_string().replace(" ", ""),
                        };

                        quote! {
                            map.insert(#flag_name.to_string(), #default_str.to_string());
                        }
                    } else {
                        quote! {}
                    }
                });
                quote! { #(#statements)* }
            }
            _ => quote! {},
        },
        _ => quote! {},
    };

    quote! {
        #[automatically_derived]
        impl crate::config::LocketDocDefaults for #struct_name {
            fn register_defaults(map: &mut std::collections::HashMap<String, String>) {
                #body
            }
        }
    }
}

fn generate_try_from_body(data: &Data) -> proc_macro2::TokenStream {
    match data {
        Data::Struct(struct_data) => match &struct_data.fields {
            Fields::Named(fields) => {
                let recurse = fields.named.iter().filter_map(|f| {
                    let name = &f.ident;

                    let should_skip = f.attrs.iter().any(|a|
                        a.path().is_ident("locket") &&
                        a.parse_args::<Meta>().is_ok_and(|m| matches!(m, Meta::Path(p) if p.is_ident("skip")))
                    );
                    if should_skip { return None; }

                    let has_default = f.attrs.iter().any(|a|
                        a.path().is_ident("locket") &&
                        a.parse_args::<Meta>().is_ok_and(|m| matches!(m, Meta::NameValue(nv) if nv.path.is_ident("default")))
                    );
                    let is_explicit_optional = f.attrs.iter().any(|a|
                        a.path().is_ident("locket") &&
                        a.parse_args::<Meta>().is_ok_and(|m| matches!(m, Meta::Path(p) if p.is_ident("optional")))
                    );

                    let is_option_type = if let Type::Path(tp) = &f.ty {
                        tp.path.segments.last().map(|s| s.ident == "Option").unwrap_or(false)
                    } else { false };

                    if !is_option_type {
                        let is_flattened = ["command", "clap", "arg", "serde"]
                            .iter()
                            .any(|key| has_attribute(&f.attrs, key, "flatten"));

                        let force_try = f.attrs.iter().any(|attr| {
                            attr.path().is_ident("locket") &&
                            attr.parse_nested_meta(|meta| {
                                if meta.path.is_ident("try_into") { Ok(()) } else { Err(meta.error("unsupported")) }
                            }).is_ok()
                        });

                        let conversion = if is_flattened || force_try {
                            quote!(.try_into()?)
                        } else {
                            quote!()
                        };

                        return Some(quote_spanned! {f.span()=>
                            #name: args.#name #conversion
                        });
                    }

                    if is_explicit_optional {
                        Some(quote_spanned! {f.span()=>
                            #name: args.#name
                        })
                    } else if has_default {
                        Some(quote_spanned! {f.span()=>
                            #name: args.#name.expect(concat!("Locket: Default logic failed for ", stringify!(#name)))
                        })
                    } else {
                        // Required Field Validation
                        let flag_name = get_clap_long_name(f);
                        let flag_literal = format!("--{}", flag_name);
                        let err_msg = format!("Missing required configuration field: {}", flag_literal);

                        Some(quote_spanned! {f.span()=>
                            #name: args.#name.ok_or_else(|| crate::config::ConfigError::Validation(#err_msg.into()))?
                        })
                    }
                });
                quote! { #(#recurse),* }
            }
            _ => quote! {},
        },
        _ => quote! {},
    }
}

fn validate_attributes(data: &Data) -> syn::Result<()> {
    let Data::Struct(data) = data else {
        return Ok(());
    };
    let Fields::Named(fields) = &data.fields else {
        return Ok(());
    };

    for field in &fields.named {
        let has_clap_flatten = ["command", "clap", "arg"]
            .iter()
            .any(|k| has_attribute(&field.attrs, k, "flatten"));
        let has_serde_flatten = has_attribute(&field.attrs, "serde", "flatten");

        let is_exempt = field.attrs.iter().any(|a| {
            a.path().is_ident("locket")
                && a.parse_args::<Meta>().is_ok_and(
                    |m| matches!(m, Meta::Path(p) if p.is_ident("allow_mismatched_flatten")),
                )
        });

        if !is_exempt && (has_clap_flatten != has_serde_flatten) {
            return Err(syn::Error::new(
                field.span(),
                "Locket: Mismatched flattening! You have `flatten` on Clap or Serde but not both.\n\
                 Fix: Ensure both attributes are present or use #[locket(allow_mismatched_flatten)].",
            ));
        }

        let has_clap_default = ["clap", "arg"].iter().any(|k| {
            has_attribute(&field.attrs, k, "default_value")
                || has_attribute(&field.attrs, k, "default_value_t")
        });

        if has_clap_default {
            return Err(syn::Error::new(
                field.span(),
                "Locket: Clap default detected! Do not use `default_value` in Clap.\n\
                 It populates the value before the config file is read, preventing overrides.\n\
                 Fix: Use `#[locket(default = ...)]` instead.",
            ));
        }
    }
    Ok(())
}

fn has_attribute(attrs: &[Attribute], path_ident: &str, nested_ident: &str) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident(path_ident) {
            return false;
        }
        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(nested_ident) {
                found = true;
            }
            Ok(())
        });
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
            if meta.path.is_ident("long")
                && let Ok(value) = meta.value()
                && let Ok(lit) = value.parse::<LitStr>()
            {
                explicit_name = Some(lit.value());
            }

            Ok(())
        });

        if let Some(name) = explicit_name {
            return name;
        }
    }
    default_name
}
