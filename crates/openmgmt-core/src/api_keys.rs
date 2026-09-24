//! API-key authentication for the MCP HTTP transport (issue #38).
//!
//! Agent/machine clients cannot complete the interactive OAuth flow (DCR +
//! PKCE + loopback redirect), so the MCP server accepts long-lived API keys
//! as an alternative credential type. Keys are:
//!
//! - 256-bit random values rendered `omg_live_<base64url>`, so they are
//!   recognizable in logs and configuration;
//! - stored as SHA-256 hashes — the plaintext is shown once, at creation,
//!   and never touches the database;
//! - scoped (`tasks:read` vs `tasks:write`) for least privilege;
//! - revocable; revoked keys fail validation.
//!
//! Audit rows record the key id, never the key. Key rows are local to the
//! serving replica and never synced.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::db::{Database, Result, parse_time};

/// Prefix rendered on every plaintext API key. The MCP HTTP auth middleware
/// routes Bearer <redacted> starting with this prefix to API-key validation;
/// everything else keeps going through `AccountAuth` unchanged.
pub const API_KEY_PREFIX: &str = "omg_live_";

/// Entropy per key, in bytes.
const API_KEY_ENTROPY_BYTES: usize = 32;
/// Characters of the encoded secret (after the prefix) kept in `key_prefix`
/// so a key can be identified in logs without revealing it.
const KEY_PREFIX_REVEAL_CHARS: usize = 7;

/// Least-privilege scopes for API keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyScope {
    /// Read tools only (`list_tasks`, `query_tasks`, …).
    TasksRead,
    /// Non-destructive write tools (`create_task`, `update_task`, …).
    /// Implies read.
    TasksWrite,
}

impl ApiKeyScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TasksRead => "tasks:read",
            Self::TasksWrite => "tasks:write",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "tasks:read" => Some(Self::TasksRead),
            "tasks:write" => Some(Self::TasksWrite),
            _ => None,
        }
    }

    /// Write scope implies read; there is no write-without-read key.
    pub fn allows_write(self) -> bool {
        matches!(self, Self::TasksWrite)
    }
}

/// True when any granted scope permits write tools.
pub fn scopes_allow_write(scopes: &[ApiKeyScope]) -> bool {
    scopes.iter().any(|scope| scope.allows_write())
}

/// Public view of a stored API key. The hash is deliberately absent: nothing
/// outside this module can recover it, and listings never leak it.
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    /// Non-secret identifier (`omg_live_` + a few characters) for logs.
    pub key_prefix: String,
    pub scopes: Vec<ApiKeyScope>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ApiKey {
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// Outcome of validating a presented API key.
#[derive(Debug)]
pub enum ApiKeyValidation {
    /// Active key; the caller is authenticated with these scopes.
    Valid(ApiKey),
    /// The key exists but was revoked; the id is safe for audit rows.
    Revoked { id: String },
    /// No key with this hash exists.
    Unknown,
}

/// Generate a fresh key: `(plaintext, sha256_hex, key_prefix)`.
fn generate_key() -> (String, String, String) {
    let mut entropy = [0u8; API_KEY_ENTROPY_BYTES];
    rand::rng().fill_bytes(&mut entropy);
    let encoded = URL_SAFE_NO_PAD.encode(entropy);
    let plaintext = format!("{API_KEY_PREFIX}{encoded}");
    let hash = format!("{:x}", Sha256::digest(plaintext.as_bytes()));
    // The encoded part is base64url ASCII, so byte slicing is safe.
    let key_prefix = plaintext[..API_KEY_PREFIX.len() + KEY_PREFIX_REVEAL_CHARS].to_string();
    (plaintext, hash, key_prefix)
}

impl Database {
    /// Create an API key, returning the public record and the plaintext
    /// secret. The plaintext is shown once — it is never stored.
    pub fn create_api_key(&self, name: &str, scopes: &[ApiKeyScope]) -> Result<(ApiKey, String)> {
        let name = name.trim();
        if name.is_empty() {
            return Err(crate::db::CoreError::Validation(
                "API key name is required".into(),
            ));
        }
        if scopes.is_empty() {
            return Err(crate::db::CoreError::Validation(
                "at least one scope is required".into(),
            ));
        }
        let (plaintext, hash, key_prefix) = generate_key();
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339();
        let scopes_json =
            serde_json::to_string(&scopes.iter().map(ApiKeyScope::as_str).collect::<Vec<_>>())
                .expect("scopes serialize");
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO mcp_api_keys (id, name, key_hash, key_prefix, scopes, created_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![id, name, hash, key_prefix, scopes_json, created_at],
        )?;
        let key = ApiKey {
            id,
            name: name.to_string(),
            key_prefix,
            scopes: scopes.to_vec(),
            created_at: parse_time(created_at)?,
            revoked_at: None,
        };
        Ok((key, plaintext))
    }

    /// List all keys (including revoked), newest first. Hashes are never
    /// exposed — [`ApiKey`] has no hash field.
    pub fn list_api_keys(&self) -> Result<Vec<ApiKey>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, created_at, key_prefix, name, scopes, key_hash, revoked_at
             FROM mcp_api_keys
             ORDER BY created_at DESC",
        )?;
        // Column order here differs from row_to_api_key; map explicitly.
        let rows = statement.query_map([], |row| {
            let scopes_raw: String = row.get(4)?;
            let scopes = serde_json::from_str::<Vec<String>>(&scopes_raw)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|scope| ApiKeyScope::parse(&scope))
                .collect();
            let revoked_at: Option<String> = row.get(6)?;
            Ok(ApiKey {
                id: row.get(0)?,
                name: row.get(3)?,
                key_prefix: row.get(2)?,
                scopes,
                created_at: parse_time(row.get(1)?)?,
                revoked_at: revoked_at.map(parse_time).transpose()?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(crate::db::CoreError::Database)
    }

    /// Revoke a key. Returns true when an active key was revoked; false when
    /// the id is unknown or already revoked.
    pub fn revoke_api_key(&self, id: &str) -> Result<bool> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE mcp_api_keys SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
            rusqlite::params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(changed == 1)
    }

    /// Validate a presented key by SHA-256 hash lookup.
    pub fn validate_api_key(&self, plaintext: &str) -> Result<ApiKeyValidation> {
        let hash = format!("{:x}", Sha256::digest(plaintext.as_bytes()));
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, created_at, key_prefix, name, scopes, revoked_at
             FROM mcp_api_keys WHERE key_hash = ?1",
        )?;
        let mut rows = statement.query_map([hash], |row| {
            let scopes_raw: String = row.get(4)?;
            let scopes = serde_json::from_str::<Vec<String>>(&scopes_raw)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|scope| ApiKeyScope::parse(&scope))
                .collect();
            let revoked_at: Option<String> = row.get(5)?;
            Ok(ApiKey {
                id: row.get(0)?,
                name: row.get(3)?,
                key_prefix: row.get(2)?,
                scopes,
                created_at: parse_time(row.get(1)?)?,
                revoked_at: revoked_at.map(parse_time).transpose()?,
            })
        })?;
        match rows.next() {
            None => Ok(ApiKeyValidation::Unknown),
            Some(Ok(key)) if key.is_revoked() => Ok(ApiKeyValidation::Revoked { id: key.id }),
            Some(Ok(key)) => Ok(ApiKeyValidation::Valid(key)),
            Some(Err(error)) => Err(crate::db::CoreError::Database(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::in_memory().unwrap()
    }

    fn hash_of(plaintext: &str) -> String {
        format!("{:x}", Sha256::digest(plaintext.as_bytes()))
    }

    #[test]
    fn create_returns_prefixed_secret_and_stores_only_its_hash() {
        let db = test_db();
        let (key, plaintext) = db
            .create_api_key("agent", &[ApiKeyScope::TasksWrite])
            .unwrap();
        assert!(plaintext.starts_with(API_KEY_PREFIX));
        assert!(key.key_prefix.starts_with(API_KEY_PREFIX));
        assert!(plaintext.starts_with(&key.key_prefix));
        assert!(key.key_prefix.len() < plaintext.len());

        let connection = db.connection().unwrap();
        let stored_hash: String = connection
            .query_row(
                "SELECT key_hash FROM mcp_api_keys WHERE id = ?1",
                [&key.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_hash, hash_of(&plaintext));
        assert_ne!(stored_hash, plaintext);
        // The plaintext appears nowhere in the table.
        let leaked: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM mcp_api_keys
                 WHERE key_hash = ?1 OR key_prefix = ?2",
                rusqlite::params![plaintext, plaintext],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leaked, 0);
    }

    #[test]
    fn generated_keys_do_not_collide() {
        let db = test_db();
        let (_, first) = db.create_api_key("a", &[ApiKeyScope::TasksRead]).unwrap();
        let (_, second) = db.create_api_key("b", &[ApiKeyScope::TasksRead]).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn validate_round_trips_scopes() {
        let db = test_db();
        let (key, plaintext) = db
            .create_api_key("agent", &[ApiKeyScope::TasksRead, ApiKeyScope::TasksWrite])
            .unwrap();
        match db.validate_api_key(&plaintext).unwrap() {
            ApiKeyValidation::Valid(valid) => {
                assert_eq!(valid.id, key.id);
                assert_eq!(valid.name, "agent");
                assert_eq!(
                    valid.scopes,
                    vec![ApiKeyScope::TasksRead, ApiKeyScope::TasksWrite]
                );
            }
            other => panic!("expected Valid, got {other:?}"),
        }
    }

    #[test]
    fn validate_unknown_key() {
        let db = test_db();
        db.create_api_key("agent", &[ApiKeyScope::TasksRead])
            .unwrap();
        assert!(matches!(
            db.validate_api_key("omg_live_doesnotexist000000000000000000000")
                .unwrap(),
            ApiKeyValidation::Unknown
        ));
        // Non-key-shaped input also misses cleanly.
        assert!(matches!(
            db.validate_api_key("not-a-key").unwrap(),
            ApiKeyValidation::Unknown
        ));
    }

    #[test]
    fn revoked_key_fails_validation_with_its_id() {
        let db = test_db();
        let (key, plaintext) = db
            .create_api_key("agent", &[ApiKeyScope::TasksRead])
            .unwrap();
        assert!(db.revoke_api_key(&key.id).unwrap());
        // Revoking twice reports nothing left to revoke.
        assert!(!db.revoke_api_key(&key.id).unwrap());
        match db.validate_api_key(&plaintext).unwrap() {
            ApiKeyValidation::Revoked { id } => assert_eq!(id, key.id),
            other => panic!("expected Revoked, got {other:?}"),
        }
    }

    #[test]
    fn revoke_unknown_id_returns_false() {
        let db = test_db();
        assert!(!db.revoke_api_key("no-such-id").unwrap());
    }

    #[test]
    fn list_shows_metadata_including_revoked_without_hashes() {
        let db = test_db();
        let (first, _) = db
            .create_api_key("first", &[ApiKeyScope::TasksRead])
            .unwrap();
        let (second, _) = db
            .create_api_key("second", &[ApiKeyScope::TasksWrite])
            .unwrap();
        db.revoke_api_key(&second.id).unwrap();

        let keys = db.list_api_keys().unwrap();
        assert_eq!(keys.len(), 2);
        // ApiKey has no hash field by construction; confirm both rows read back.
        let ids: Vec<&str> = keys.iter().map(|key| key.id.as_str()).collect();
        assert!(ids.contains(&first.id.as_str()));
        assert!(ids.contains(&second.id.as_str()));
        let revoked = keys.iter().find(|key| key.id == second.id).unwrap();
        assert!(revoked.is_revoked());
        let active = keys.iter().find(|key| key.id == first.id).unwrap();
        assert!(!active.is_revoked());
        assert_eq!(active.scopes, vec![ApiKeyScope::TasksRead]);
    }

    #[test]
    fn create_rejects_empty_name_and_empty_scopes() {
        let db = test_db();
        assert!(db.create_api_key("", &[ApiKeyScope::TasksRead]).is_err());
        assert!(db.create_api_key("   ", &[ApiKeyScope::TasksRead]).is_err());
        assert!(db.create_api_key("agent", &[]).is_err());
    }

    #[test]
    fn scope_parse_round_trip() {
        assert_eq!(
            ApiKeyScope::parse("tasks:read"),
            Some(ApiKeyScope::TasksRead)
        );
        assert_eq!(
            ApiKeyScope::parse("tasks:write"),
            Some(ApiKeyScope::TasksWrite)
        );
        assert_eq!(ApiKeyScope::parse("admin"), None);
        assert_eq!(ApiKeyScope::parse(""), None);
        assert!(!scopes_allow_write(&[ApiKeyScope::TasksRead]));
        assert!(scopes_allow_write(&[
            ApiKeyScope::TasksRead,
            ApiKeyScope::TasksWrite
        ]));
        assert!(scopes_allow_write(&[ApiKeyScope::TasksWrite]));
        assert!(!scopes_allow_write(&[]));
        assert!(ApiKeyScope::TasksWrite.allows_write());
        assert!(!ApiKeyScope::TasksRead.allows_write());
    }
}
