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
            #[group(id = $group_id, multiple = false, required = false)]
            pub struct $struct_name {
                #[arg(
                    long = concat!($prefix, ".token"),
                    env = $env,
                    hide_env_values = true,
                    help = $doc_string
                )]
                [< $prefix _val >]: Option<secrecy::SecretString>,

                #[arg(
                    long = concat!($prefix, ".token-file"),
                    env = concat!($env, "_FILE"),
                    help = concat!("Path to file containing ", $doc_string)
                )]
                [< $prefix _file >]: Option<std::path::PathBuf>,
            }

            impl TryFrom<$struct_name> for crate::provider::AuthToken {
                type Error = crate::provider::ProviderError;

                fn try_from(value: $struct_name) -> Result<Self, Self::Error> {
                    crate::provider::AuthToken::try_new(
                        value.[< $prefix _val >],
                        value.[< $prefix _file >],
                        $doc_string,
                    )
                }
            }
        }
    };
}

pub(super) use define_auth_token;
