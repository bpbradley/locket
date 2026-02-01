use super::{ProviderError, ReferenceParser, SecretReference, SecretsProvider, Signature};
use async_trait::async_trait;
use secrecy::SecretString;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A factory trait that creates specific backend clients from configuration.
#[async_trait]
pub trait ProviderFactory: Signature + ReferenceParser + Send + Sync + Sized + Clone {
    async fn create(&self) -> Result<Arc<dyn SecretsProvider>, ProviderError>;
}

/// A wrapper that handles automatic rotation of the underlying provider.
pub struct ManagedProvider<C> {
    config: C,
    state: RwLock<ProviderState>,
}

struct ProviderState {
    inner: Arc<dyn SecretsProvider>,
    signature: u64,
}

impl<C> ManagedProvider<C>
where
    C: ProviderFactory + 'static,
{
    pub async fn new(config: C) -> Result<Self, ProviderError> {
        let signature = config.signature().await?;
        let inner = config.create().await?;
        Ok(Self {
            config,
            state: RwLock::new(ProviderState { inner, signature }),
        })
    }
}

#[async_trait]
impl<C> SecretsProvider for ManagedProvider<C>
where
    C: ProviderFactory + 'static,
{
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
        {
            let state = self.state.read().await;
            match state.inner.fetch_map(references).await {
                Ok(res) => return Ok(res),
                Err(_) => {
                    // Fallthrough to rotation logic
                    // don't propagate the error yet.
                }
            }
        }

        // Check signature of the config to see if it has changed
        let new_signature = match self.config.signature().await {
            Ok(s) => s,
            Err(e) => return Err(e), // can't check signature, fail hard.
        };

        // upgrade lock
        let mut state = self.state.write().await;

        // Check again in case another task already rotated
        if state.signature != new_signature {
            // The config has changed, which may be the cause of the prior failure.
            // Rebuild the inner provider and swap it in.
            let new_inner = match self.config.create().await {
                Ok(bg) => bg,
                Err(e) => return Err(e), // Failed to rebuild
            };

            state.inner = new_inner;
            state.signature = new_signature;
        }

        // Retry the fetch with the (possibly) new inner provider

        let inner = state.inner.clone();
        drop(state);

        inner.fetch_map(references).await
    }
}

impl<C> ReferenceParser for ManagedProvider<C>
where
    C: ProviderFactory + 'static,
{
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        self.config.parse(raw)
    }
}
