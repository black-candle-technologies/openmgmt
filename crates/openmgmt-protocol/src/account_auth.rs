//! Black Candle account authentication for the sync protocol.
//!
//! Device registration is gated on a Black Candle access token. The token is
//! validated against the issuer's `/oauth/userinfo` endpoint (authd exposes no
//! introspection or JWKS endpoint, so userinfo is the validation call), and
//! device/account ownership is keyed by the stable user id — never the email.
//!
//! This module is shared by the sync server (registration gate) and the
//! HTTPS MCP transport (bearer auth), so both enforce the same identity.

use reqwest::StatusCode;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long a successful userinfo validation is cached, keyed by token hash.
const USERINFO_CACHE_TTL: Duration = Duration::from_secs(300);

/// Validated Black Candle account identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    /// Stable authd user id. This is the ownership key.
    pub user_id: String,
    pub email: String,
    pub email_verified: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AccountAuthError {
    #[error("invalid or expired access token")]
    InvalidToken,
    #[error("account email is not verified")]
    EmailUnverified,
    #[error("account issuer error: {0}")]
    Issuer(String),
}

#[derive(Debug, Deserialize)]
struct UserInfoResponse {
    id: String,
    email: String,
    email_verified: bool,
}

/// Validates Black Candle access tokens against an OAuth issuer's
/// `/oauth/userinfo` endpoint.
pub struct AccountAuth {
    userinfo_url: String,
    client: reqwest::Client,
    cache: Mutex<HashMap<String, (AccountIdentity, Instant)>>,
}

impl AccountAuth {
    /// `issuer` is the base URL of the OAuth issuer, e.g.
    /// `https://auth.blackcandletech.com`. It is configurable so
    /// self-hosters can point at their own issuer.
    pub fn new(issuer: &str) -> Result<Self, AccountAuthError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            // Bundled Mozilla roots: the server ships as a distroless
            // container with no system CA store.
            .build()
            .map_err(|error| AccountAuthError::Issuer(error.to_string()))?;
        Ok(Self {
            userinfo_url: format!("{}/oauth/userinfo", issuer.trim_end_matches('/')),
            client,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Validate a bearer access token, returning the account identity.
    /// Successful validations are cached for five minutes by token hash.
    pub async fn validate(&self, bearer_token: &str) -> Result<AccountIdentity, AccountAuthError> {
        let cache_key = format!("{:x}", Sha256::digest(bearer_token.as_bytes()));
        {
            let cache = self
                .cache
                .lock()
                .map_err(|_| AccountAuthError::Issuer("cache lock poisoned".into()))?;
            if let Some((identity, validated_at)) = cache.get(&cache_key)
                && validated_at.elapsed() < USERINFO_CACHE_TTL
            {
                return Ok(identity.clone());
            }
        }

        let response = self
            .client
            .get(&self.userinfo_url)
            .bearer_auth(bearer_token)
            .send()
            .await
            .map_err(|error| AccountAuthError::Issuer(error.to_string()))?;
        let identity = match response.status() {
            StatusCode::OK => {
                let info: UserInfoResponse = response
                    .json()
                    .await
                    .map_err(|error| AccountAuthError::Issuer(error.to_string()))?;
                if !info.email_verified {
                    return Err(AccountAuthError::EmailUnverified);
                }
                AccountIdentity {
                    user_id: info.id,
                    email: info.email,
                    email_verified: true,
                }
            }
            // authd returns 401 for invalid/expired tokens and 403 for
            // unverified email.
            StatusCode::UNAUTHORIZED => return Err(AccountAuthError::InvalidToken),
            StatusCode::FORBIDDEN => return Err(AccountAuthError::EmailUnverified),
            status => {
                return Err(AccountAuthError::Issuer(format!(
                    "userinfo endpoint returned {status}"
                )));
            }
        };
        self.cache
            .lock()
            .map_err(|_| AccountAuthError::Issuer("cache lock poisoned".into()))?
            .insert(cache_key, (identity.clone(), Instant::now()));
        Ok(identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_a_stable_hex_digest() {
        let first = format!("{:x}", Sha256::digest("token-abc".as_bytes()));
        let second = format!("{:x}", Sha256::digest("token-abc".as_bytes()));
        let other = format!("{:x}", Sha256::digest("token-abd".as_bytes()));
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn userinfo_url_trims_trailing_slash() {
        let auth = AccountAuth::new("https://auth.example.com/").unwrap();
        assert_eq!(auth.userinfo_url, "https://auth.example.com/oauth/userinfo");
    }
}
