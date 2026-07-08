//! Shared token lifecycle for providers that exchange a long lived
//! credential for a short lived client token.
//!
//! `TokenAuthenticator` caches the current token behind a `RwLock`,
//! renews it through the provider's `TokenExchange` when it expires, and
//! supports poisoning so a token the server rejected is replaced even if
//! it has not reached its deadline yet.

use super::ProviderError;
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::debug;

/// Tokens are renewed this long before they actually expire, so a token
/// is never used within moments of its deadline.
const EXPIRY_LEEWAY: Duration = Duration::from_secs(60);

/// A credential exchange that produces a fresh client token.
#[async_trait]
pub trait TokenExchange: Send + Sync {
    async fn login(&self) -> Result<ExpiringToken, ProviderError>;
}

/// Caches a client token and lazily renews it when it expires or is
/// invalidated.
///
/// The token is held in a `RwLock` to allow concurrent reads and
/// exclusive writes.
pub struct TokenAuthenticator<E: TokenExchange> {
    exchange: E,
    token: RwLock<ExpiringToken>,
}

impl<E: TokenExchange> TokenAuthenticator<E> {
    /// Performs an initial login so a bad credential fails at
    /// construction rather than on first fetch.
    pub async fn try_new(exchange: E) -> Result<Self, ProviderError> {
        let token = exchange.login().await?;

        Ok(Self {
            exchange,
            token: RwLock::new(token),
        })
    }

    /// Returns a valid client token, renewing it if necessary.
    pub async fn get_token(&self) -> Result<SecretString, ProviderError> {
        {
            let guard = self.token.read().await;
            if !guard.is_expired() {
                return Ok(guard.secret.clone());
            }
        }

        // Token expired. Need to renew
        let mut guard = self.token.write().await;

        // Check if token is expired again in case it was renewed by
        // another task while waiting for the write lock
        if !guard.is_expired() {
            return Ok(guard.secret.clone());
        }

        debug!("Token expired. Renewing...");
        let token = self.exchange.login().await?;
        let secret = token.secret.clone();
        *guard = token;

        Ok(secret)
    }

    /// Marks the given token as expired if it is still the cached one,
    /// forcing the next `get_token` to renew.
    pub async fn invalidate(&self, token: &SecretString) {
        let mut guard = self.token.write().await;
        if guard.secret.expose_secret() == token.expose_secret() {
            guard.poison();
        }
    }
}

/// A client token together with its expiry.
pub struct ExpiringToken {
    secret: SecretString,
    expiry: TokenExpiry,
}

impl ExpiringToken {
    pub fn new(secret: SecretString, lease_duration_secs: u64) -> Self {
        let expiry = TokenExpiry::from_lease_duration(lease_duration_secs);
        match expiry {
            TokenExpiry::Never => debug!("Acquired auth token. Token does not expire"),
            TokenExpiry::At(_) => debug!(
                "Acquired auth token. Expires in {} seconds",
                lease_duration_secs
            ),
        }
        Self { secret, expiry }
    }

    fn is_expired(&self) -> bool {
        self.expiry.is_expired()
    }

    fn poison(&mut self) {
        // Set to a point in the past so that it will be considered expired.
        // Applies to non-expiring tokens too: they can still be revoked
        // server side, and poisoning must force a fresh login.
        self.expiry = TokenExpiry::At(Instant::now() - Duration::from_secs(1));
    }
}

#[derive(Clone, Copy)]
enum TokenExpiry {
    Never,
    At(Instant),
}

impl TokenExpiry {
    /// Vault style semantics: a lease duration of `0` means the token
    /// never expires, e.g. an AppRole role with `token_ttl=0`.
    fn from_lease_duration(seconds: u64) -> Self {
        match seconds {
            0 => Self::Never,
            s => Instant::now()
                .checked_add(Duration::from_secs(s))
                .map_or(Self::Never, Self::At),
        }
    }

    fn is_expired(&self) -> bool {
        match self {
            Self::Never => false,
            Self::At(deadline) => *deadline <= Instant::now() + EXPIRY_LEEWAY,
        }
    }
}

/// Serializes a borrowed `SecretString` as a plain string, for login
/// payloads that must carry the credential in the request body.
pub struct SecretView<'a>(pub &'a SecretString);

impl Serialize for SecretView<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.expose_secret())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_with_expiry(expiry: TokenExpiry) -> ExpiringToken {
        ExpiringToken {
            secret: SecretString::new("test-token".into()),
            expiry,
        }
    }

    #[test]
    fn test_token_not_expired_well_before_leeway() {
        let token = token_with_expiry(TokenExpiry::At(Instant::now() + Duration::from_secs(120)));
        assert!(!token.is_expired());
    }

    #[test]
    fn test_token_expired_within_leeway() {
        // is_expired() treats tokens expiring within EXPIRY_LEEWAY as expired
        // already, so renewal happens before the token actually stops working.
        let token = token_with_expiry(TokenExpiry::At(Instant::now() + EXPIRY_LEEWAY / 2));
        assert!(token.is_expired());
    }

    #[test]
    fn test_token_expired_in_the_past() {
        let token = token_with_expiry(TokenExpiry::At(Instant::now() - Duration::from_secs(1)));
        assert!(token.is_expired());
    }

    #[test]
    fn test_token_poison_marks_expired() {
        let mut token =
            token_with_expiry(TokenExpiry::At(Instant::now() + Duration::from_secs(120)));
        assert!(!token.is_expired());
        token.poison();
        assert!(token.is_expired());
    }

    #[test]
    fn test_zero_lease_duration_never_expires() {
        let token = ExpiringToken::new(SecretString::new("test-token".into()), 0);
        assert!(!token.is_expired());
    }

    #[test]
    fn test_nonzero_lease_duration_expires() {
        assert!(matches!(
            TokenExpiry::from_lease_duration(900),
            TokenExpiry::At(_)
        ));
    }

    #[test]
    fn test_poison_forces_expiry_of_non_expiring_token() {
        let mut token = token_with_expiry(TokenExpiry::Never);
        assert!(!token.is_expired());
        token.poison();
        assert!(token.is_expired());
    }

    #[test]
    fn test_absurd_lease_duration_does_not_panic() {
        // A hostile or broken server could report a lease that overflows
        // Instant arithmetic. Saturate to Never instead of panicking.
        let token = token_with_expiry(TokenExpiry::from_lease_duration(u64::MAX));
        assert!(!token.is_expired());
    }
}
