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

/// Default Black Candle auth issuer, shared by the sync server
/// (`OPENMGMT_AUTH_ISSUER`) and the MCP HTTP transport
/// (`OPENMGMT_MCP_AUTH_ISSUER`). Self-hosters point both at their own
/// authd-compatible issuer.
pub const DEFAULT_AUTH_ISSUER: &str = "https://auth.blackcandletech.com";

/// Validated Black Candle account identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    /// Stable authd user id. This is the ownership key.
    pub user_id: String,
    pub email: String,
    pub email_verified: bool,
    /// OAuth scopes granted to the token, space-delimited per RFC 6749
    /// (e.g. `"identity"`, or `"openmgmt:tasks:read"` for a personal
    /// access token minted with a narrow scope). Reported by authd's
    /// `/oauth/userinfo`. Issuers that predate scoped tokens omit the
    /// field; it defaults to `"identity"` (full access), preserving the
    /// historical behavior where any valid token could do anything.
    /// Scope is immutable for a given token string, so caching it with
    /// the identity is safe.
    pub scope: String,
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
    // authd's userinfo returns the user id as a JSON number (SQLite
    // rowid); accept either form so a numeric id never breaks token
    // validation with "error decoding response body".
    #[serde(deserialize_with = "de_string_or_int")]
    id: String,
    email: String,
    email_verified: bool,
    // Absent on issuers that predate scoped tokens; the default keeps
    // back-compat (full access, like every token before scopes existed).
    #[serde(default = "default_scope")]
    scope: String,
}

/// Scope assumed when the issuer does not report one: the legacy
/// full-access scope every pre-scope token effectively carried.
fn default_scope() -> String {
    "identity".to_string()
}

/// Deserializes a string that may arrive as a JSON string or integer
/// (authd returns numeric user ids).
fn de_string_or_int<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct StringOrInt;

    impl Visitor<'_> for StringOrInt {
        type Value = String;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or integer")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
            Ok(value.to_owned())
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<String, E> {
            Ok(value.to_string())
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<String, E> {
            Ok(value.to_string())
        }
    }

    deserializer.deserialize_any(StringOrInt)
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
                    scope: info.scope,
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

    #[test]
    fn userinfo_reports_scope_when_present() {
        let scoped: UserInfoResponse = serde_json::from_str(
            r#"{"id": 7, "email": "a@b.c", "email_verified": true, "scope": "openmgmt:tasks:read"}"#,
        )
        .expect("scoped userinfo must deserialize");
        assert_eq!(scoped.scope, "openmgmt:tasks:read");

        let multi: UserInfoResponse = serde_json::from_str(
            r#"{"id": 7, "email": "a@b.c", "email_verified": true, "scope": "openmgmt:tasks:read courier:messages:read"}"#,
        )
        .expect("multi-scope userinfo must deserialize");
        assert_eq!(multi.scope, "openmgmt:tasks:read courier:messages:read");
    }

    #[test]
    fn userinfo_defaults_scope_to_identity_when_absent() {
        // Pre-scope issuers omit the field; the default preserves the
        // historical behavior where any valid token had full access.
        let legacy: UserInfoResponse =
            serde_json::from_str(r#"{"id": 7, "email": "a@b.c", "email_verified": true}"#)
                .expect("legacy userinfo must deserialize");
        assert_eq!(legacy.scope, "identity");
    }

    #[test]
    fn userinfo_accepts_numeric_or_string_id() {
        // Regression test: authd returns the user id as a JSON number.
        // A strict `id: String` field rejected it with
        // "error decoding response body", turning every device
        // registration into HTTP 500.
        let numeric: UserInfoResponse =
            serde_json::from_str(r#"{"id": 12345, "email": "a@b.c", "email_verified": true}"#)
                .expect("numeric id must deserialize");
        assert_eq!(numeric.id, "12345");

        let textual: UserInfoResponse =
            serde_json::from_str(r#"{"id": "67890", "email": "a@b.c", "email_verified": true}"#)
                .expect("string id must deserialize");
        assert_eq!(textual.id, "67890");
    }
}
