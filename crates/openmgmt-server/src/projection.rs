//! Persistent account-scoped read projection. Server ingestion owns reduction; clients never replay events.
use crate::store::{Result, StoreError};
use chrono::{DateTime, Utc};
use openmgmt_core::{Project, ProjectStatus, Task};
use openmgmt_protocol::{SyncEntityType, SyncEvent, SyncOperation};
use rusqlite::{Connection, params};

pub fn migrate(connection: &mut Connection) -> Result<()> {
    let tx = connection.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS mobile_projection (
        account_id TEXT NOT NULL,kind TEXT NOT NULL,id TEXT NOT NULL,title TEXT NOT NULL,
        description TEXT NOT NULL,status TEXT NOT NULL,updated_at TEXT NOT NULL,search_text TEXT NOT NULL DEFAULT '',
        archived INTEGER NOT NULL,valid INTEGER NOT NULL,revision INTEGER NOT NULL,
        PRIMARY KEY(account_id,kind,id));
        CREATE INDEX IF NOT EXISTS mobile_projection_query_idx ON mobile_projection(account_id,kind,archived,valid,updated_at,id);")?;
    let initialized: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM server_state WHERE key='mobile_projection_v1')",
        [],
        |row| row.get(0),
    )?;
    if !initialized {
        let accounts: Vec<String> = tx
            .prepare("SELECT DISTINCT account_id FROM server_devices WHERE account_id IS NOT NULL")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for account in accounts {
            rebuild_account(&tx, &account)?;
        }
        tx.execute(
            "INSERT INTO server_state(key,value) VALUES ('mobile_projection_v1','1')",
            [],
        )?;
    }
    // A persisted random key authenticates opaque cursors across process restarts, never transmitted.
    tx.execute(
        "INSERT OR IGNORE INTO server_state(key,value) VALUES ('mobile_cursor_key',?1)",
        [format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        )],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn rebuild_account(connection: &Connection, account: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM mobile_projection WHERE account_id=?1",
        [account],
    )?;
    let mut statement = connection.prepare("SELECT e.rowid,e.event_id,e.device_id,e.entity_type,e.entity_id,e.operation,e.payload_json,e.created_at,e.sequence
        FROM server_events e JOIN server_devices d ON d.device_id=e.device_id WHERE d.account_id=?1 ORDER BY e.rowid")?;
    let mut rows = statement.query([account])?;
    while let Some(row) = rows.next()? {
        let payload: String = row.get(6)?;
        let timestamp: String = row.get(7)?;
        let kind: String = row.get(3)?;
        let operation: String = row.get(5)?;
        let event = SyncEvent {
            event_id: row.get(1)?,
            device_id: row.get(2)?,
            actor_user_id: None,
            target_user_id: None,
            workspace_id: None,
            sequence: row.get(8)?,
            entity_type: kind.parse().map_err(StoreError::RegistrationDenied)?,
            entity_id: row.get(4)?,
            operation: operation.parse().map_err(StoreError::RegistrationDenied)?,
            payload_json: serde_json::from_str(&payload).unwrap_or_default(),
            created_at: DateTime::parse_from_rfc3339(&timestamp)
                .map(|v| v.with_timezone(&Utc))
                .unwrap_or_default(),
            synced_at: None,
        };
        apply(connection, account, &event, row.get(0)?)?;
    }
    Ok(())
}

pub fn apply(
    connection: &Connection,
    account: &str,
    event: &SyncEvent,
    revision: i64,
) -> Result<()> {
    let kind = match event.entity_type {
        SyncEntityType::Task => "task",
        SyncEntityType::Project => "project",
        _ => return Ok(()),
    };
    if event.operation == SyncOperation::Archived {
        let archive_id = event.payload_json.get("id").and_then(|v| v.as_str());
        let timestamp = event
            .payload_json
            .get("archived_at")
            .and_then(|v| v.as_str())
            .and_then(|v| DateTime::parse_from_rfc3339(v).ok());
        if archive_id != Some(event.entity_id.as_str()) || timestamp.is_none() {
            return invalidate(connection, account, kind, event, revision);
        }
        // An archive without a full snapshot remains undisplayable; do not fabricate an item.
        connection.execute("INSERT INTO mobile_projection(account_id,kind,id,title,description,status,updated_at,archived,valid,revision)
            VALUES (?1,?2,?3,'','','archived',?4,1,0,?5) ON CONFLICT(account_id,kind,id) DO UPDATE SET
            status=CASE WHEN ?2='task' THEN 'canceled' ELSE 'archived' END,updated_at=excluded.updated_at,archived=1,revision=excluded.revision",
            params![account,kind,event.entity_id,timestamp.unwrap().with_timezone(&Utc).to_rfc3339_opts(chrono::SecondsFormat::Nanos,true),revision])?;
        return Ok(());
    }
    let snapshot = event
        .payload_json
        .get("entity")
        .cloned()
        .unwrap_or_default();
    if event.payload_json.to_string().len() > 65536
        || (kind == "project" && event.operation == SyncOperation::Transitioned)
    {
        return invalidate(connection, account, kind, event, revision);
    }
    let projected = if kind == "task" {
        serde_json::from_value::<Task>(snapshot).ok().map(|task| {
            (
                task.id,
                task.title,
                task.description.unwrap_or_default(),
                task.status.to_string(),
                task.updated_at,
                false,
            )
        })
    } else {
        serde_json::from_value::<Project>(snapshot)
            .ok()
            .map(|project| {
                (
                    project.id,
                    project.name,
                    project.description.unwrap_or_default(),
                    project.status.to_string(),
                    project.updated_at,
                    project.archived_at.is_some() || project.status == ProjectStatus::Archived,
                )
            })
    };
    let Some((id, title, description, status, updated, archived)) = projected else {
        return invalidate(connection, account, kind, event, revision);
    };
    if id != event.entity_id
        || id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || title.len() > 1024
        || description.len() > 16384
    {
        return invalidate(connection, account, kind, event, revision);
    }
    connection.execute("INSERT INTO mobile_projection(account_id,kind,id,title,description,status,updated_at,archived,valid,revision,search_text)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,?9,?10) ON CONFLICT(account_id,kind,id) DO UPDATE SET title=excluded.title,
        description=excluded.description,status=excluded.status,updated_at=excluded.updated_at,archived=excluded.archived,valid=1,revision=excluded.revision,search_text=excluded.search_text",
        params![account,kind,id,title,description,status,updated.to_rfc3339_opts(chrono::SecondsFormat::Nanos,true),archived,revision,
            format!("{} {}",title.to_lowercase(),description.to_lowercase())])?;
    Ok(())
}

fn invalidate(
    connection: &Connection,
    account: &str,
    kind: &str,
    event: &SyncEvent,
    revision: i64,
) -> Result<()> {
    connection.execute("INSERT INTO mobile_projection(account_id,kind,id,title,description,status,updated_at,archived,valid,revision)
        VALUES (?1,?2,?3,'','','',?4,0,0,?5) ON CONFLICT(account_id,kind,id) DO UPDATE SET valid=0,revision=excluded.revision",
        params![account,kind,event.entity_id,event.created_at.to_rfc3339(),revision])?;
    Ok(())
}
