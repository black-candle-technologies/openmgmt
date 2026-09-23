//! Background sync-outbox publisher.
//!
//! The MCP server shares its SQLite file with the sync server. Core records
//! every mutation in the local `sync_events` outbox, but the MCP has no sync
//! client pushing that outbox over HTTP — so without this task, MCP-created
//! tasks, projects, and edits would never reach syncing devices.
//!
//! The publisher periodically copies unsynced outbox rows into the
//! server-side event log (`Database::publish_sync_outbox`) and ensures the
//! local device is registered under the deployment's account so
//! account-scoped sync pulls include its events
//! (`Database::ensure_server_device`). Both are no-ops when the database is
//! not a shared server database.

use openmgmt_core::Database;
use std::time::Duration;
use tokio::task::JoinHandle;

const DEFAULT_INTERVAL_SECS: u64 = 15;

/// Spawn a background task that publishes this process's sync outbox.
/// The task runs until the runtime shuts down; the handle is returned so
/// callers can keep it alive.
pub fn spawn_sync_publisher(database: Database) -> JoinHandle<()> {
    tokio::spawn(async move {
        let interval_secs = std::env::var("OPENMGMT_MCP_SYNC_PUBLISH_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_INTERVAL_SECS);
        let account_id = std::env::var("OPENMGMT_MCP_SYNC_ACCOUNT_ID").ok();
        if account_id.is_none() {
            tracing::warn!(
                "OPENMGMT_MCP_SYNC_ACCOUNT_ID is not set: published events may be \
                 hidden from account-scoped sync pulls"
            );
        }
        tracing::info!(
            interval_secs,
            account_configured = account_id.is_some(),
            "sync publisher started"
        );
        loop {
            let database = database.clone();
            let account_id = account_id.clone();
            let cycle = tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
                if let Some(account_id) = account_id.as_deref() {
                    database.ensure_server_device(account_id, "OpenMGMT MCP")?;
                }
                Ok(database.publish_sync_outbox()?)
            })
            .await;
            match cycle {
                Ok(Ok(0)) => {}
                Ok(Ok(published)) => tracing::info!(
                    published,
                    "sync publisher: published outbox events to the server event log"
                ),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "sync publisher: publish cycle failed")
                }
                Err(join_error) => {
                    tracing::warn!(%join_error, "sync publisher: worker task failed")
                }
            }
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
        }
    })
}
