use chrono::{DateTime, SecondsFormat, Utc};
use openmgmt_protocol::{
    AuthContext, DeviceRegistrationRequest, ProtocolError, ProtocolErrorCode, RejectedSyncEvent,
    SyncEvent,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::{
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("database lock poisoned")]
    LockPoisoned,
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredDevice {
    pub device_id: String,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub device_token: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PushResult {
    pub accepted_event_ids: Vec<String>,
    pub rejected_events: Vec<RejectedSyncEvent>,
    pub server_checkpoint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullPage {
    pub events: Vec<SyncEvent>,
    pub server_checkpoint: String,
    pub has_more: bool,
}

#[derive(Clone)]
pub struct ServerStore {
    connection: Arc<Mutex<Connection>>,
}

impl ServerStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let store = Self {
            connection: Arc::new(Mutex::new(Connection::open_in_memory()?)),
        };
        store.migrate()?;
        Ok(store)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| StoreError::LockPoisoned)
    }

    pub fn migrate(&self) -> Result<()> {
        self.connection()?.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS server_events (
              event_id TEXT PRIMARY KEY NOT NULL,
              device_id TEXT NOT NULL,
              actor_user_id TEXT,
              target_user_id TEXT,
              workspace_id TEXT,
              sequence INTEGER NOT NULL,
              entity_type TEXT NOT NULL,
              entity_id TEXT NOT NULL,
              operation TEXT NOT NULL,
              payload_json TEXT NOT NULL,
              created_at TEXT NOT NULL,
              received_at TEXT NOT NULL,
              UNIQUE(device_id, sequence)
            );
            CREATE INDEX IF NOT EXISTS server_events_checkpoint_idx
              ON server_events(received_at, event_id);
            CREATE INDEX IF NOT EXISTS server_events_entity_idx
              ON server_events(entity_type, entity_id);
            CREATE TABLE IF NOT EXISTS server_devices (
              device_id TEXT PRIMARY KEY NOT NULL,
              device_name TEXT NOT NULL,
              account_id TEXT,
              user_id TEXT,
              device_token TEXT,
              created_at TEXT NOT NULL,
              last_seen_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS server_state (
              key TEXT PRIMARY KEY NOT NULL,
              value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn table_exists(&self, table: &str) -> Result<bool> {
        Ok(self.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )?)
    }

    pub fn register_device(&self, request: &DeviceRegistrationRequest) -> Result<RegisteredDevice> {
        let now = server_timestamp(Utc::now());
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing = transaction
            .query_row(
                "SELECT account_id,user_id,device_token FROM server_devices WHERE device_id=?1",
                [&request.device_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let (account_id, user_id, device_token) = match existing {
            Some((account_id, user_id, token)) => (
                account_id,
                user_id,
                token.unwrap_or_else(|| Uuid::new_v4().to_string()),
            ),
            None => (None, None, Uuid::new_v4().to_string()),
        };
        transaction.execute(
            "INSERT INTO server_devices (
                device_id,device_name,account_id,user_id,device_token,created_at,last_seen_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?6)
             ON CONFLICT(device_id) DO UPDATE SET
                device_name=excluded.device_name,
                device_token=excluded.device_token,
                last_seen_at=excluded.last_seen_at",
            params![
                request.device_id,
                request.device_name,
                account_id,
                user_id,
                device_token,
                now
            ],
        )?;
        transaction.commit()?;
        Ok(RegisteredDevice {
            device_id: request.device_id.clone(),
            account_id,
            user_id,
            device_token,
        })
    }

    pub fn authenticate(&self, auth: &AuthContext) -> Result<bool> {
        let connection = self.connection()?;
        let token = connection
            .query_row(
                "SELECT device_token FROM server_devices WHERE device_id=?1",
                [&auth.device_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        let Some(token) = token else {
            return Ok(false);
        };
        if token.as_deref() != auth.device_token.as_deref() {
            return Ok(false);
        }
        connection.execute(
            "UPDATE server_devices SET last_seen_at=?2 WHERE device_id=?1",
            params![auth.device_id, server_timestamp(Utc::now())],
        )?;
        Ok(true)
    }

    pub fn push_events(&self, events: &[SyncEvent]) -> Result<PushResult> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut accepted_event_ids = Vec::new();
        let mut rejected_events = Vec::new();

        for event in events {
            if transaction
                .query_row(
                    "SELECT 1 FROM server_events WHERE event_id=?1",
                    [&event.event_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()
            {
                accepted_event_ids.push(event.event_id.clone());
                continue;
            }

            if let Some(existing_event_id) = transaction
                .query_row(
                    "SELECT event_id FROM server_events WHERE device_id=?1 AND sequence=?2",
                    params![event.device_id, event.sequence],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            {
                rejected_events.push(RejectedSyncEvent {
                    event_id: event.event_id.clone(),
                    error: ProtocolError::new(
                        ProtocolErrorCode::Conflict,
                        format!("device sequence already belongs to event {existing_event_id}"),
                        false,
                    ),
                });
                continue;
            }

            transaction.execute(
                "INSERT INTO server_events (
                    event_id,device_id,actor_user_id,target_user_id,workspace_id,sequence,
                    entity_type,entity_id,operation,payload_json,created_at,received_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    event.event_id,
                    event.device_id,
                    event.actor_user_id,
                    event.target_user_id,
                    event.workspace_id,
                    event.sequence,
                    event.entity_type.to_string(),
                    event.entity_id,
                    event.operation.to_string(),
                    event.payload_json.to_string(),
                    server_timestamp(event.created_at),
                    server_timestamp(Utc::now()),
                ],
            )?;
            accepted_event_ids.push(event.event_id.clone());
        }

        let server_checkpoint = latest_checkpoint_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(PushResult {
            accepted_event_ids,
            rejected_events,
            server_checkpoint,
        })
    }

    pub fn pull_events(
        &self,
        after: Option<(DateTime<Utc>, String)>,
        limit: u32,
    ) -> Result<PullPage> {
        let connection = self.connection()?;
        let query_limit = i64::from(limit) + 1;
        let mut events = if let Some((received_at, event_id)) = after {
            let mut statement = connection.prepare(&format!(
                "{SERVER_EVENT_SELECT}
                     WHERE received_at > ?1 OR (received_at = ?1 AND event_id > ?2)
                     ORDER BY received_at,event_id LIMIT ?3"
            ))?;
            statement
                .query_map(
                    params![server_timestamp(received_at), event_id, query_limit],
                    map_server_event,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut statement = connection.prepare(&format!(
                "{SERVER_EVENT_SELECT} ORDER BY received_at,event_id LIMIT ?1"
            ))?;
            statement
                .query_map([query_limit], map_server_event)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let has_more = events.len() > limit as usize;
        if has_more {
            events.pop();
        }
        let server_checkpoint = if let Some(last) = events.last() {
            make_checkpoint(last.received_at, &last.event.event_id)
        } else {
            latest_checkpoint_with_connection(&connection)?
        };
        Ok(PullPage {
            events: events.into_iter().map(|stored| stored.event).collect(),
            server_checkpoint,
            has_more,
        })
    }
}

#[derive(Debug)]
struct StoredEvent {
    event: SyncEvent,
    received_at: DateTime<Utc>,
}

const SERVER_EVENT_SELECT: &str =
    "SELECT event_id,device_id,actor_user_id,target_user_id,workspace_id,sequence,
     entity_type,entity_id,operation,payload_json,created_at,received_at FROM server_events";

fn map_server_event(row: &Row<'_>) -> rusqlite::Result<StoredEvent> {
    let payload_json = row.get::<_, String>(9)?;
    Ok(StoredEvent {
        event: SyncEvent {
            event_id: row.get(0)?,
            device_id: row.get(1)?,
            actor_user_id: row.get(2)?,
            target_user_id: row.get(3)?,
            workspace_id: row.get(4)?,
            sequence: row.get(5)?,
            entity_type: parse_enum(row.get::<_, String>(6)?, 6)?,
            entity_id: row.get(7)?,
            operation: parse_enum(row.get::<_, String>(8)?, 8)?,
            payload_json: serde_json::from_str(&payload_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            created_at: parse_time(row.get(10)?, 10)?,
            synced_at: None,
        },
        received_at: parse_time(row.get(11)?, 11)?,
    })
}

fn latest_checkpoint_with_connection(connection: &Connection) -> Result<String> {
    let latest = connection
        .query_row(
            "SELECT received_at,event_id FROM server_events
             ORDER BY received_at DESC,event_id DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    latest
        .map(|(received_at, event_id)| {
            parse_time(received_at, 0).map(|received_at| make_checkpoint(received_at, &event_id))
        })
        .transpose()
        .map(|value| value.unwrap_or_default())
        .map_err(StoreError::from)
}

fn parse_enum<T>(value: String, column: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value.parse::<T>().map_err(|error| {
        let error = std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string());
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn parse_time(value: String, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

fn server_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

pub fn make_checkpoint(received_at: DateTime<Utc>, event_id: &str) -> String {
    format!("{}|{event_id}", server_timestamp(received_at))
}

pub fn parse_checkpoint(value: &str) -> Option<(DateTime<Utc>, String)> {
    let (received_at, event_id) = value.split_once('|')?;
    let received_at = DateTime::parse_from_rfc3339(received_at)
        .ok()?
        .with_timezone(&Utc);
    (!event_id.is_empty()).then(|| (received_at, event_id.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmgmt_protocol::DeviceRegistrationRequest;

    #[test]
    fn migration_creates_server_tables() {
        let store = ServerStore::in_memory().unwrap();
        for table in ["server_events", "server_devices", "server_state"] {
            assert!(store.table_exists(table).unwrap(), "missing {table}");
        }
    }

    #[test]
    fn device_registration_creates_and_preserves_token() {
        let store = ServerStore::in_memory().unwrap();
        let request = DeviceRegistrationRequest {
            protocol_version: "omgp/1".into(),
            device_id: "device-1".into(),
            device_name: "Desktop".into(),
            user_hint: None,
        };

        let first = store.register_device(&request).unwrap();
        let second = store.register_device(&request).unwrap();

        assert!(!first.device_token.is_empty());
        assert_eq!(first.device_token, second.device_token);
    }

    #[test]
    fn checkpoint_roundtrips() {
        let received_at = Utc::now();
        let checkpoint = make_checkpoint(received_at, "event-1");
        let parsed = parse_checkpoint(&checkpoint).unwrap();
        assert_eq!(parsed.0, received_at);
        assert_eq!(parsed.1, "event-1");
    }
}
