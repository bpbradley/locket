/// Macro to define an authentication token struct with Clap arguments
/// for either direct value or file path.
/// This is necessary to avoid boilerplate for a pattern that each
/// provider is likely to need. And Clap lacks prefix support on
/// flattened structs.
macro_rules! define_auth_token {
    (
        struct_name: $struct_name:ident,
        prefix: $prefix:literal,
        env: $env:literal,
        group_id: $group_id:literal,
        doc_string: $doc_string:literal
    ) => {
        paste::paste! {
            #[derive(clap::Args, Debug, Clone, Default)]
            #[group(id = $group_id, required = false, multiple = false)]
            pub struct $struct_name {
                #[arg(
                    long = concat!($prefix, ".token"),
                    env = $env,
                    hide_env_values = true,
                    group = $group_id,
                    help = $doc_string
                )]
                pub [< $prefix _val >]: Option<secrecy::SecretString>,

                #[arg(
                    long = concat!($prefix, ".token-file"),
                    env = concat!($env, "_FILE"),
                    group = $group_id,
                    help = concat!("Path to file containing ", $doc_string)
                )]
                pub [< $prefix _file >]: Option<crate::path::CanonicalPath>,
            }

            impl TryFrom<$struct_name> for crate::provider::TokenSource  {
                type Error = crate::provider::ProviderError;

                fn try_from(value: $struct_name) -> Result<Self, Self::Error> {
                    if let Some(val) = value.[< $prefix _val >] {
                        Ok(crate::provider::TokenSource::Literal(val))
                    } else if let Some(canon_path) = value.[< $prefix _file >] {
                        Ok(crate::provider::TokenSource::File(canon_path))
                    } else {
                        Err(crate::provider::ProviderError::InvalidConfig(
                            format!("{}: either {}.token or {}.token-file must be provided", $doc_string, $prefix, $prefix)
                        ))
                    }
                }
            }

            impl TryFrom<$struct_name> for crate::provider::AuthToken {
                type Error = crate::provider::ProviderError;

                fn try_from(value: $struct_name) -> Result<Self, Self::Error> {
                    let source: crate::provider::TokenSource = value.try_into()?;
                    crate::provider::AuthToken::try_from_source(source, $doc_string)
                }
            }
        }
    };
}

pub(super) use define_auth_token;
