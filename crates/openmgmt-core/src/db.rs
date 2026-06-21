use crate::{
    board::build_board,
    models::{
        BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch, Project,
        ProjectPatch, ProjectStatus, ProjectType, Task, TaskContext, TaskPatch, TaskStatus,
    },
    sync::{
        ConflictPolicy, DEFAULT_DEVICE_NAME, LOCAL_DEVICE_ID_KEY, RemoteApplyBatchResult,
        RemoteApplyResult, RemoteApplyStatus, SYNC_ACCOUNT_ID_KEY, SYNC_DEVICE_NAME_KEY,
        SYNC_DEVICE_TOKEN_KEY, SYNC_ENABLED_KEY, SYNC_LAST_ATTEMPTED_AT_KEY, SYNC_LAST_ERROR_KEY,
        SYNC_LAST_SUCCESSFUL_AT_KEY, SYNC_SERVER_URL_KEY, SYNC_USER_ID_KEY, SyncConflict,
        SyncConflictKind, SyncConflictPolicyAction, SyncConflictResolutionStatus,
        SyncConnectionState, SyncEntityType, SyncEvent, SyncOperation, SyncSettings,
        SyncSettingsPatch, SyncStatus,
    },
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("invalid stored value: {0}")]
    InvalidValue(String),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("database lock poisoned")]
    LockPoisoned,
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationOrigin {
    Local,
    #[allow(dead_code)]
    Remote,
    Seed,
}

impl MutationOrigin {
    fn logs_sync_event(self) -> bool {
        matches!(self, Self::Local)
    }
}

pub fn default_database_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OPENMGMT_DATABASE_PATH") {
        return PathBuf::from(path);
    }
    if let Ok(current) = std::env::current_dir() {
        for ancestor in current.ancestors() {
            let manifest = ancestor.join("Cargo.toml");
            if std::fs::read_to_string(&manifest).is_ok_and(|text| text.contains("[workspace]")) {
                return ancestor.join("data").join("openmgmt.sqlite");
            }
        }
    }
    PathBuf::from("data").join("openmgmt.sqlite")
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let database = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        database.migrate()?;
        Ok(database)
    }

    pub fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let database = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        database.migrate()?;
        Ok(database)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| CoreError::LockPoisoned)
    }

    pub fn migrate(&self) -> Result<()> {
        self.connection()?.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS organizations (
              id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, slug TEXT NOT NULL UNIQUE,
              description TEXT, color TEXT, icon TEXT, created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL, archived_at TEXT
            );
            CREATE TABLE IF NOT EXISTS projects (
              id TEXT PRIMARY KEY NOT NULL,
              organization_id TEXT NOT NULL REFERENCES organizations(id),
              name TEXT NOT NULL, slug TEXT NOT NULL, description TEXT, type TEXT NOT NULL,
              status TEXT NOT NULL, priority INTEGER NOT NULL, deadline TEXT, repo_url TEXT,
              notes TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, archived_at TEXT,
              UNIQUE(organization_id, slug)
            );
            CREATE INDEX IF NOT EXISTS projects_organization_idx ON projects(organization_id);
            CREATE TABLE IF NOT EXISTS tasks (
              id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL REFERENCES projects(id),
              title TEXT NOT NULL, description TEXT, status TEXT NOT NULL, priority INTEGER NOT NULL,
              due_at TEXT, scheduled_at TEXT, started_at TEXT, completed_at TEXT,
              estimated_minutes INTEGER, time_limit_minutes INTEGER, pinned INTEGER NOT NULL DEFAULT 0,
              blocked_reason TEXT, tags TEXT NOT NULL DEFAULT '[]',
              created_at TEXT NOT NULL, updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS tasks_project_idx ON tasks(project_id);
            CREATE INDEX IF NOT EXISTS tasks_status_idx ON tasks(status);
            CREATE INDEX IF NOT EXISTS tasks_due_idx ON tasks(due_at);
            CREATE TABLE IF NOT EXISTS sync_events (
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
              synced_at TEXT,
              UNIQUE(device_id, sequence)
            );
            CREATE INDEX IF NOT EXISTS sync_events_unsynced_idx ON sync_events(synced_at);
            CREATE INDEX IF NOT EXISTS sync_events_entity_idx
              ON sync_events(entity_type, entity_id);
            CREATE TABLE IF NOT EXISTS sync_state (
              key TEXT PRIMARY KEY NOT NULL,
              value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sync_devices (
              device_id TEXT PRIMARY KEY NOT NULL,
              name TEXT NOT NULL,
              created_at TEXT NOT NULL,
              last_seen_at TEXT
            );
            CREATE TABLE IF NOT EXISTS applied_remote_events (
              event_id TEXT PRIMARY KEY NOT NULL,
              device_id TEXT NOT NULL,
              actor_user_id TEXT,
              target_user_id TEXT,
              workspace_id TEXT,
              sequence INTEGER NOT NULL,
              entity_type TEXT NOT NULL,
              entity_id TEXT NOT NULL,
              operation TEXT NOT NULL,
              applied_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS applied_remote_events_entity_idx
              ON applied_remote_events(entity_type, entity_id);
            CREATE INDEX IF NOT EXISTS applied_remote_events_device_sequence_idx
              ON applied_remote_events(device_id, sequence);
            CREATE TABLE IF NOT EXISTS sync_conflicts (
              conflict_id TEXT PRIMARY KEY NOT NULL,
              remote_event_id TEXT NOT NULL,
              local_device_id TEXT NOT NULL,
              entity_type TEXT NOT NULL,
              entity_id TEXT NOT NULL,
              conflict_kind TEXT NOT NULL,
              policy_action TEXT NOT NULL,
              local_snapshot_json TEXT,
              remote_snapshot_json TEXT,
              resolution_status TEXT NOT NULL,
              created_at TEXT NOT NULL,
              resolved_at TEXT
            );
            CREATE INDEX IF NOT EXISTS sync_conflicts_entity_idx
              ON sync_conflicts(entity_type, entity_id);
            CREATE INDEX IF NOT EXISTS sync_conflicts_status_idx
              ON sync_conflicts(resolution_status, created_at);
            CREATE UNIQUE INDEX IF NOT EXISTS sync_conflicts_remote_event_idx
              ON sync_conflicts(remote_event_id);
            "#,
        )?;
        self.ensure_sync_event_ownership_columns()?;
        Ok(())
    }

    fn ensure_sync_event_ownership_columns(&self) -> Result<()> {
        let connection = self.connection()?;
        let columns = connection
            .prepare("PRAGMA table_info(sync_events)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for column in ["actor_user_id", "target_user_id", "workspace_id"] {
            if !columns.iter().any(|existing| existing == column) {
                connection.execute(
                    &format!("ALTER TABLE sync_events ADD COLUMN {column} TEXT"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    pub fn get_or_create_device_id(&self) -> Result<String> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let device_id = get_or_create_device_id_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(device_id)
    }

    pub fn append_sync_event(
        &self,
        entity_type: SyncEntityType,
        entity_id: &str,
        operation: SyncOperation,
        payload_json: serde_json::Value,
    ) -> Result<SyncEvent> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let event = append_sync_event_with_connection(
            &transaction,
            entity_type,
            entity_id,
            operation,
            payload_json,
        )?;
        transaction.commit()?;
        Ok(event)
    }

    pub fn list_unsynced_events(&self) -> Result<Vec<SyncEvent>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT event_id,device_id,actor_user_id,target_user_id,workspace_id,sequence,
                    entity_type,entity_id,operation,payload_json,created_at,synced_at
             FROM sync_events WHERE synced_at IS NULL ORDER BY device_id,sequence",
        )?;
        Ok(statement
            .query_map([], map_sync_event)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn mark_sync_events_synced(&self, event_ids: &[String]) -> Result<()> {
        if event_ids.is_empty() {
            return Ok(());
        }
        let synced_at = timestamp(Utc::now());
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        for event_id in event_ids {
            transaction.execute(
                "UPDATE sync_events SET synced_at=?2 WHERE event_id=?1",
                params![event_id, synced_at],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn has_applied_remote_event(&self, event_id: &str) -> Result<bool> {
        Ok(self.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM applied_remote_events WHERE event_id=?1)",
            [event_id],
            |row| row.get(0),
        )?)
    }

    pub fn list_sync_conflicts(&self) -> Result<Vec<SyncConflict>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT conflict_id,remote_event_id,local_device_id,entity_type,entity_id,
                    conflict_kind,policy_action,local_snapshot_json,remote_snapshot_json,
                    resolution_status,created_at,resolved_at
             FROM sync_conflicts ORDER BY created_at DESC",
        )?;
        Ok(statement
            .query_map([], map_sync_conflict)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_open_sync_conflicts(&self) -> Result<Vec<SyncConflict>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT conflict_id,remote_event_id,local_device_id,entity_type,entity_id,
                    conflict_kind,policy_action,local_snapshot_json,remote_snapshot_json,
                    resolution_status,created_at,resolved_at
             FROM sync_conflicts WHERE resolution_status='open' ORDER BY created_at DESC",
        )?;
        Ok(statement
            .query_map([], map_sync_conflict)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn mark_sync_conflict_ignored(&self, conflict_id: &str) -> Result<SyncConflict> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        changed(
            transaction.execute(
                "UPDATE sync_conflicts
                 SET resolution_status='ignored',resolved_at=?2
                 WHERE conflict_id=?1",
                params![conflict_id, timestamp(Utc::now())],
            )?,
            "sync conflict",
        )?;
        let conflict = get_sync_conflict_with_connection(&transaction, conflict_id)?;
        transaction.commit()?;
        Ok(conflict)
    }

    pub fn apply_remote_sync_event(&self, event: &SyncEvent) -> Result<RemoteApplyResult> {
        self.apply_remote_sync_event_with_policy(event, &ConflictPolicy::default())
    }

    pub fn apply_remote_sync_event_with_policy(
        &self,
        event: &SyncEvent,
        policy: &ConflictPolicy,
    ) -> Result<RemoteApplyResult> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let local_device_id = get_or_create_device_id_with_connection(&transaction)?;
        let (status, conflict_ids, auto_resolved_conflict_count) =
            if event.device_id == local_device_id {
                (RemoteApplyStatus::SkippedLocalEcho, Vec::new(), 0)
            } else if transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM applied_remote_events WHERE event_id=?1)",
                [&event.event_id],
                |row| row.get::<_, bool>(0),
            )? {
                (RemoteApplyStatus::AlreadyApplied, Vec::new(), 0)
            } else {
                let conflict_outcome = apply_remote_domain_change_with_policy(
                    &transaction,
                    event,
                    policy,
                    &local_device_id,
                )?;
                transaction.execute(
                    "INSERT INTO applied_remote_events (
                    event_id,device_id,actor_user_id,target_user_id,workspace_id,sequence,
                    entity_type,entity_id,operation,applied_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
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
                        timestamp(Utc::now()),
                    ],
                )?;
                (
                    RemoteApplyStatus::Applied,
                    conflict_outcome.conflict_ids,
                    conflict_outcome.auto_resolved_conflict_count,
                )
            };
        transaction.commit()?;
        Ok(RemoteApplyResult {
            event_id: event.event_id.clone(),
            entity_type: event.entity_type,
            entity_id: event.entity_id.clone(),
            operation: event.operation,
            status,
            conflict_ids,
            auto_resolved_conflict_count,
        })
    }

    pub fn apply_remote_sync_events(&self, events: &[SyncEvent]) -> Result<RemoteApplyBatchResult> {
        self.apply_remote_sync_events_with_policy(events, &ConflictPolicy::default())
    }

    pub fn apply_remote_sync_events_with_policy(
        &self,
        events: &[SyncEvent],
        policy: &ConflictPolicy,
    ) -> Result<RemoteApplyBatchResult> {
        let mut result = RemoteApplyBatchResult {
            applied_count: 0,
            already_applied_count: 0,
            skipped_local_echo_count: 0,
            conflict_count: 0,
            auto_resolved_conflict_count: 0,
            results: Vec::with_capacity(events.len()),
        };
        let mut ordered_events = events.iter().collect::<Vec<_>>();
        ordered_events.sort_by_key(|event| remote_apply_dependency_rank(event.entity_type));
        for event in ordered_events {
            let applied = self.apply_remote_sync_event_with_policy(event, policy)?;
            match applied.status {
                RemoteApplyStatus::Applied => result.applied_count += 1,
                RemoteApplyStatus::AlreadyApplied => result.already_applied_count += 1,
                RemoteApplyStatus::SkippedLocalEcho => result.skipped_local_echo_count += 1,
            }
            result.conflict_count += applied.conflict_ids.len();
            result.auto_resolved_conflict_count += applied.auto_resolved_conflict_count;
            result.results.push(applied);
        }
        Ok(result)
    }

    pub fn get_sync_state(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection()?
            .query_row("SELECT value FROM sync_state WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn set_sync_state(&self, key: &str, value: &str) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO sync_state (key,value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_sync_settings(&self) -> Result<SyncSettings> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let (_, settings) = get_sync_settings_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(settings)
    }

    pub fn update_sync_settings(&self, patch: SyncSettingsPatch) -> Result<SyncSettings> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let (device_id, mut settings) = get_sync_settings_with_connection(&transaction)?;

        if let Some(enabled) = patch.enabled {
            settings.enabled = enabled;
        }
        if let Some(server_url) = patch.server_url {
            settings.server_url = normalize_optional(server_url);
        }
        if let Some(device_name) = patch.device_name {
            let device_name = device_name.trim();
            settings.device_name = if device_name.is_empty() {
                DEFAULT_DEVICE_NAME.into()
            } else {
                device_name.into()
            };
        }
        if let Some(account_id) = patch.account_id {
            settings.account_id = normalize_optional(account_id);
        }
        if let Some(user_id) = patch.user_id {
            settings.user_id = normalize_optional(user_id);
        }
        if let Some(device_token) = patch.device_token {
            settings.device_token = normalize_optional(device_token);
        }
        validate_server_url(settings.server_url.as_deref())?;

        set_sync_state_with_connection(
            &transaction,
            SYNC_ENABLED_KEY,
            if settings.enabled { "true" } else { "false" },
        )?;
        set_optional_sync_state_with_connection(
            &transaction,
            SYNC_SERVER_URL_KEY,
            settings.server_url.as_deref(),
        )?;
        set_sync_state_with_connection(&transaction, SYNC_DEVICE_NAME_KEY, &settings.device_name)?;
        set_optional_sync_state_with_connection(
            &transaction,
            SYNC_ACCOUNT_ID_KEY,
            settings.account_id.as_deref(),
        )?;
        set_optional_sync_state_with_connection(
            &transaction,
            SYNC_USER_ID_KEY,
            settings.user_id.as_deref(),
        )?;
        set_optional_sync_state_with_connection(
            &transaction,
            SYNC_DEVICE_TOKEN_KEY,
            settings.device_token.as_deref(),
        )?;
        transaction.execute(
            "UPDATE sync_devices SET name=?2 WHERE device_id=?1",
            params![device_id, settings.device_name],
        )?;
        transaction.commit()?;
        Ok(settings)
    }

    pub fn get_sync_status(&self) -> Result<SyncStatus> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let status = get_sync_status_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(status)
    }

    pub fn record_sync_attempt_started(&self) -> Result<SyncStatus> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        set_sync_state_with_connection(
            &transaction,
            SYNC_LAST_ATTEMPTED_AT_KEY,
            &timestamp(Utc::now()),
        )?;
        let status = get_sync_status_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(status)
    }

    pub fn record_sync_success(&self) -> Result<SyncStatus> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let now = timestamp(Utc::now());
        set_sync_state_with_connection(&transaction, SYNC_LAST_SUCCESSFUL_AT_KEY, &now)?;
        set_sync_state_with_connection(&transaction, SYNC_LAST_ATTEMPTED_AT_KEY, &now)?;
        set_optional_sync_state_with_connection(&transaction, SYNC_LAST_ERROR_KEY, None)?;
        let status = get_sync_status_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(status)
    }

    pub fn record_sync_error(&self, error: &str) -> Result<SyncStatus> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        set_sync_state_with_connection(
            &transaction,
            SYNC_LAST_ATTEMPTED_AT_KEY,
            &timestamp(Utc::now()),
        )?;
        set_sync_state_with_connection(&transaction, SYNC_LAST_ERROR_KEY, error)?;
        let status = get_sync_status_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(status)
    }

    pub fn clear_sync_error(&self) -> Result<SyncStatus> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        set_optional_sync_state_with_connection(&transaction, SYNC_LAST_ERROR_KEY, None)?;
        let status = get_sync_status_with_connection(&transaction)?;
        transaction.commit()?;
        Ok(status)
    }

    pub fn list_organizations(&self) -> Result<Vec<Organization>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,name,slug,description,color,icon,created_at,updated_at,archived_at
             FROM organizations WHERE archived_at IS NULL ORDER BY name",
        )?;
        Ok(statement
            .query_map([], map_organization)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_organization(&self, input: NewOrganization) -> Result<Organization> {
        self.create_organization_internal(input, MutationOrigin::Local)
    }

    fn create_organization_internal(
        &self,
        input: NewOrganization,
        origin: MutationOrigin,
    ) -> Result<Organization> {
        require_name(&input.name)?;
        let now = Utc::now();
        let organization = Organization {
            id: Uuid::new_v4().to_string(),
            slug: slugify(input.slug.as_deref().unwrap_or(&input.name)),
            name: input.name,
            description: input.description,
            color: input.color,
            icon: input.icon,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO organizations VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                organization.id,
                organization.name,
                organization.slug,
                organization.description,
                organization.color,
                organization.icon,
                timestamp(organization.created_at),
                timestamp(organization.updated_at),
                Option::<String>::None
            ],
        )?;
        if origin.logs_sync_event() {
            append_sync_event_with_connection(
                &transaction,
                SyncEntityType::Organization,
                &organization.id,
                SyncOperation::Created,
                serde_json::json!({ "entity": &organization }),
            )?;
        }
        transaction.commit()?;
        Ok(organization)
    }

    pub fn update_organization(&self, id: &str, patch: OrganizationPatch) -> Result<Organization> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut current = get_organization_with_connection(&transaction, id)?;
        if let Some(name) = patch.name {
            require_name(&name)?;
            current.name = name;
        }
        if let Some(slug) = patch.slug {
            current.slug = slugify(&slug);
        }
        if let Some(value) = patch.description {
            current.description = value;
        }
        if let Some(value) = patch.color {
            current.color = value;
        }
        if let Some(value) = patch.icon {
            current.icon = value;
        }
        current.updated_at = Utc::now();
        transaction.execute(
            "UPDATE organizations SET name=?2,slug=?3,description=?4,color=?5,icon=?6,updated_at=?7
             WHERE id=?1",
            params![
                id,
                current.name,
                current.slug,
                current.description,
                current.color,
                current.icon,
                timestamp(current.updated_at)
            ],
        )?;
        append_sync_event_with_connection(
            &transaction,
            SyncEntityType::Organization,
            &current.id,
            SyncOperation::Updated,
            serde_json::json!({ "entity": &current }),
        )?;
        transaction.commit()?;
        Ok(current)
    }

    pub fn archive_organization(&self, id: &str) -> Result<()> {
        let archived_at = Utc::now();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        changed(
            transaction.execute(
                "UPDATE organizations SET archived_at=?2,updated_at=?2 WHERE id=?1",
                params![id, timestamp(archived_at)],
            )?,
            "organization",
        )?;
        append_sync_event_with_connection(
            &transaction,
            SyncEntityType::Organization,
            id,
            SyncOperation::Archived,
            serde_json::json!({ "id": id, "archived_at": archived_at }),
        )?;
        transaction.commit()?;
        Ok(())
    }

    #[cfg(test)]
    fn get_organization(&self, id: &str) -> Result<Organization> {
        let connection = self.connection()?;
        get_organization_with_connection(&connection, id)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT p.id,p.organization_id,p.name,p.slug,p.description,p.type,p.status,p.priority,
                    p.deadline,p.repo_url,p.notes,p.created_at,p.updated_at,p.archived_at
             FROM projects p JOIN organizations o ON o.id=p.organization_id
             WHERE p.archived_at IS NULL AND p.status != 'archived' AND o.archived_at IS NULL
             ORDER BY p.priority DESC,p.name",
        )?;
        Ok(statement
            .query_map([], map_project)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_project(&self, id: &str) -> Result<Project> {
        let connection = self.connection()?;
        get_project_with_connection(&connection, id)
    }

    pub fn create_project(&self, input: NewProject) -> Result<Project> {
        self.create_project_internal(input, MutationOrigin::Local)
    }

    fn create_project_internal(
        &self,
        input: NewProject,
        origin: MutationOrigin,
    ) -> Result<Project> {
        require_name(&input.name)?;
        validate_priority(input.priority)?;
        let now = Utc::now();
        let project = Project {
            id: Uuid::new_v4().to_string(),
            organization_id: input.organization_id,
            slug: slugify(input.slug.as_deref().unwrap_or(&input.name)),
            name: input.name,
            description: input.description,
            project_type: input.project_type,
            status: input.status,
            priority: input.priority,
            deadline: input.deadline,
            repo_url: input.repo_url,
            notes: input.notes,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        get_organization_with_connection(&transaction, &project.organization_id)?;
        transaction.execute(
            "INSERT INTO projects VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                project.id,
                project.organization_id,
                project.name,
                project.slug,
                project.description,
                project.project_type.to_string(),
                project.status.to_string(),
                project.priority,
                project.deadline.map(timestamp),
                project.repo_url,
                project.notes,
                timestamp(project.created_at),
                timestamp(project.updated_at),
                Option::<String>::None
            ],
        )?;
        if origin.logs_sync_event() {
            append_sync_event_with_connection(
                &transaction,
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Created,
                serde_json::json!({ "entity": &project }),
            )?;
        }
        transaction.commit()?;
        Ok(project)
    }

    pub fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut project = get_project_with_connection(&transaction, id)?;
        if let Some(value) = patch.name {
            require_name(&value)?;
            project.name = value;
        }
        if let Some(value) = patch.slug {
            project.slug = slugify(&value);
        }
        if let Some(value) = patch.description {
            project.description = value;
        }
        if let Some(value) = patch.project_type {
            project.project_type = value;
        }
        if let Some(value) = patch.status {
            project.status = value;
        }
        if let Some(value) = patch.priority {
            validate_priority(value)?;
            project.priority = value;
        }
        if let Some(value) = patch.deadline {
            project.deadline = value;
        }
        if let Some(value) = patch.repo_url {
            project.repo_url = value;
        }
        if let Some(value) = patch.notes {
            project.notes = value;
        }
        project.updated_at = Utc::now();
        transaction.execute(
            "UPDATE projects SET name=?2,slug=?3,description=?4,type=?5,status=?6,priority=?7,
             deadline=?8,repo_url=?9,notes=?10,updated_at=?11 WHERE id=?1",
            params![
                id,
                project.name,
                project.slug,
                project.description,
                project.project_type.to_string(),
                project.status.to_string(),
                project.priority,
                project.deadline.map(timestamp),
                project.repo_url,
                project.notes,
                timestamp(project.updated_at)
            ],
        )?;
        append_sync_event_with_connection(
            &transaction,
            SyncEntityType::Project,
            &project.id,
            SyncOperation::Updated,
            serde_json::json!({ "entity": &project }),
        )?;
        transaction.commit()?;
        Ok(project)
    }

    pub fn archive_project(&self, id: &str) -> Result<()> {
        let archived_at = Utc::now();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        changed(
            transaction.execute(
                "UPDATE projects SET status='archived',archived_at=?2,updated_at=?2 WHERE id=?1",
                params![id, timestamp(archived_at)],
            )?,
            "project",
        )?;
        append_sync_event_with_connection(
            &transaction,
            SyncEntityType::Project,
            id,
            SyncOperation::Archived,
            serde_json::json!({ "id": id, "archived_at": archived_at }),
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT t.id,t.project_id,t.title,t.description,t.status,t.priority,t.due_at,
                    t.scheduled_at,t.started_at,t.completed_at,t.estimated_minutes,
                    t.time_limit_minutes,t.pinned,t.blocked_reason,t.tags,t.created_at,t.updated_at
             FROM tasks t JOIN projects p ON p.id=t.project_id
             JOIN organizations o ON o.id=p.organization_id
             WHERE t.status != 'canceled' AND p.archived_at IS NULL
               AND p.status != 'archived' AND o.archived_at IS NULL
             ORDER BY t.priority DESC,t.created_at",
        )?;
        Ok(statement
            .query_map([], map_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_task(&self, id: &str) -> Result<Task> {
        let connection = self.connection()?;
        get_task_with_connection(&connection, id)
    }

    pub fn create_task(&self, input: NewTask) -> Result<Task> {
        self.create_task_internal(input, MutationOrigin::Local)
    }

    fn create_task_internal(&self, input: NewTask, origin: MutationOrigin) -> Result<Task> {
        require_name(&input.title)?;
        validate_priority(input.priority)?;
        let now = Utc::now();
        let task = Task {
            id: Uuid::new_v4().to_string(),
            project_id: input.project_id,
            title: input.title,
            description: input.description,
            status: input.status,
            priority: input.priority,
            due_at: input.due_at,
            scheduled_at: input.scheduled_at,
            started_at: (input.status == TaskStatus::InProgress).then_some(now),
            completed_at: None,
            estimated_minutes: input.estimated_minutes,
            time_limit_minutes: input.time_limit_minutes,
            pinned: input.pinned,
            blocked_reason: None,
            tags: input.tags,
            created_at: now,
            updated_at: now,
        };
        let status = task.status.to_string();
        let due_at = task.due_at.map(timestamp);
        let scheduled_at = task.scheduled_at.map(timestamp);
        let started_at = task.started_at.map(timestamp);
        let completed_at = task.completed_at.map(timestamp);
        let tags = serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".into());
        let created_at = timestamp(task.created_at);
        let updated_at = timestamp(task.updated_at);
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        get_project_with_connection(&transaction, &task.project_id)?;
        transaction.execute(
            "INSERT INTO tasks VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                task.id,
                task.project_id,
                task.title,
                task.description,
                status,
                task.priority,
                due_at,
                scheduled_at,
                started_at,
                completed_at,
                task.estimated_minutes,
                task.time_limit_minutes,
                task.pinned,
                task.blocked_reason,
                tags,
                created_at,
                updated_at
            ],
        )?;
        if origin.logs_sync_event() {
            append_sync_event_with_connection(
                &transaction,
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Created,
                serde_json::json!({ "entity": &task }),
            )?;
        }
        transaction.commit()?;
        Ok(task)
    }

    pub fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut task = get_task_with_connection(&transaction, id)?;
        let now = Utc::now();
        if let Some(value) = patch.title {
            require_name(&value)?;
            task.title = value;
        }
        if let Some(value) = patch.description {
            task.description = value;
        }
        if let Some(value) = patch.status {
            if value == TaskStatus::InProgress && task.started_at.is_none() {
                task.started_at = Some(now);
            }
            if value == TaskStatus::Done && task.completed_at.is_none() {
                task.completed_at = Some(now);
            }
            task.status = value;
        }
        if let Some(value) = patch.priority {
            validate_priority(value)?;
            task.priority = value;
        }
        if let Some(value) = patch.due_at {
            task.due_at = value;
        }
        if let Some(value) = patch.scheduled_at {
            task.scheduled_at = value;
        }
        if let Some(value) = patch.estimated_minutes {
            task.estimated_minutes = value;
        }
        if let Some(value) = patch.time_limit_minutes {
            task.time_limit_minutes = value;
        }
        if let Some(value) = patch.pinned {
            task.pinned = value;
        }
        if let Some(value) = patch.blocked_reason {
            task.blocked_reason = value;
        }
        if let Some(value) = patch.tags {
            task.tags = value;
        }
        task.updated_at = now;
        save_task_with_connection(&transaction, &task)?;
        append_sync_event_with_connection(
            &transaction,
            SyncEntityType::Task,
            &task.id,
            SyncOperation::Updated,
            serde_json::json!({ "entity": &task }),
        )?;
        transaction.commit()?;
        Ok(task)
    }

    pub fn transition_task(
        &self,
        id: &str,
        status: TaskStatus,
        blocked_reason: Option<String>,
    ) -> Result<Task> {
        self.transition_task_internal(id, status, blocked_reason, MutationOrigin::Local)
    }

    fn transition_task_internal(
        &self,
        id: &str,
        status: TaskStatus,
        blocked_reason: Option<String>,
        origin: MutationOrigin,
    ) -> Result<Task> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut task = get_task_with_connection(&transaction, id)?;
        let now = Utc::now();
        task.status = status;
        task.updated_at = now;
        task.blocked_reason = blocked_reason;
        if status == TaskStatus::InProgress && task.started_at.is_none() {
            task.started_at = Some(now);
        }
        if status == TaskStatus::Done {
            task.completed_at = Some(now);
        }
        save_task_with_connection(&transaction, &task)?;
        if origin.logs_sync_event() {
            append_sync_event_with_connection(
                &transaction,
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Transitioned,
                serde_json::json!({ "entity": &task }),
            )?;
        }
        transaction.commit()?;
        Ok(task)
    }

    pub fn board_state(&self) -> Result<BoardState> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT t.id,t.project_id,t.title,t.description,t.status,t.priority,t.due_at,
              t.scheduled_at,t.started_at,t.completed_at,t.estimated_minutes,t.time_limit_minutes,
              t.pinned,t.blocked_reason,t.tags,t.created_at,t.updated_at,
              p.name,p.type,p.status,p.priority,o.name,o.color
             FROM tasks t JOIN projects p ON p.id=t.project_id
             JOIN organizations o ON o.id=p.organization_id
             WHERE p.archived_at IS NULL AND p.status != 'archived' AND o.archived_at IS NULL",
        )?;
        let contexts = statement
            .query_map([], |row| {
                Ok(TaskContext {
                    task: map_task(row)?,
                    project_name: row.get(17)?,
                    project_type: parse_enum(row.get::<_, String>(18)?)?,
                    project_status: parse_enum(row.get::<_, String>(19)?)?,
                    project_priority: row.get(20)?,
                    organization_name: row.get(21)?,
                    organization_color: row.get(22)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(build_board(contexts, Utc::now()))
    }

    pub fn seed(&self) -> Result<()> {
        let organizations = [
            ("Black Candle", "black-candle", "#d85b52", "BC"),
            ("Triarii", "triarii", "#d1a33b", "TR"),
            (
                "In the National Interest",
                "in-the-national-interest",
                "#4d78d2",
                "NI",
            ),
            ("Personal", "personal", "#8a5ac2", "PS"),
        ];
        for (name, slug, color, icon) in organizations {
            let exists: bool = self.connection()?.query_row(
                "SELECT EXISTS(SELECT 1 FROM organizations WHERE slug=?1)",
                [slug],
                |row| row.get(0),
            )?;
            if !exists {
                self.create_organization_internal(
                    NewOrganization {
                        name: name.into(),
                        slug: Some(slug.into()),
                        description: None,
                        color: Some(color.into()),
                        icon: Some(icon.into()),
                    },
                    MutationOrigin::Seed,
                )?;
            }
        }
        let personal = self
            .list_organizations()?
            .into_iter()
            .find(|org| org.slug == "personal")
            .ok_or(CoreError::NotFound("seed organization"))?;
        let existing_project = {
            let connection = self.connection()?;
            connection
                .query_row(
                    "SELECT id,organization_id,name,slug,description,type,status,priority,deadline,
                        repo_url,notes,created_at,updated_at,archived_at
                 FROM projects WHERE organization_id=?1 AND slug='openmgmt'
                   AND archived_at IS NULL",
                    [&personal.id],
                    map_project,
                )
                .optional()?
        };
        let project = existing_project.map(Ok).unwrap_or_else(|| {
            self.create_project_internal(
                NewProject {
                    organization_id: personal.id,
                    name: "OpenMgmt".into(),
                    slug: Some("openmgmt".into()),
                    description: Some("Local-first project and task operations console.".into()),
                    project_type: ProjectType::Software,
                    status: ProjectStatus::Active,
                    priority: 5,
                    deadline: None,
                    repo_url: Some("https://github.com/LaneBucher/openmgmt".into()),
                    notes: None,
                },
                MutationOrigin::Seed,
            )
        })?;

        let now = Utc::now();
        let seeds = [
            (
                "Review the MVP on the TV board",
                TaskStatus::InProgress,
                5,
                Some(now + Duration::hours(2)),
                Some(now),
                true,
                None,
            ),
            (
                "Document local backup workflow",
                TaskStatus::Ready,
                3,
                Some(now + Duration::hours(20)),
                None,
                false,
                None,
            ),
            (
                "Resolve overdue launch decision",
                TaskStatus::Ready,
                4,
                Some(now - Duration::hours(3)),
                None,
                false,
                None,
            ),
            (
                "Confirm external dependency",
                TaskStatus::Blocked,
                4,
                Some(now + Duration::hours(8)),
                None,
                false,
                Some("Waiting for confirmation"),
            ),
            (
                "Plan the afternoon review",
                TaskStatus::Scheduled,
                2,
                Some(now + Duration::hours(30)),
                Some(now + Duration::hours(4)),
                false,
                None,
            ),
            (
                "Capture launch notes",
                TaskStatus::Inbox,
                2,
                None,
                None,
                false,
                None,
            ),
        ];
        for (title, status, priority, due_at, scheduled_at, pinned, blocked_reason) in seeds {
            let exists: bool = self.connection()?.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE project_id=?1 AND title=?2)",
                params![project.id, title],
                |row| row.get(0),
            )?;
            if !exists {
                let task = self.create_task_internal(
                    NewTask {
                        project_id: project.id.clone(),
                        title: title.into(),
                        description: None,
                        status,
                        priority,
                        due_at,
                        scheduled_at,
                        estimated_minutes: Some(30),
                        time_limit_minutes: Some(45),
                        pinned,
                        tags: vec!["seed".into()],
                    },
                    MutationOrigin::Seed,
                )?;
                if let Some(reason) = blocked_reason {
                    self.transition_task_internal(
                        &task.id,
                        TaskStatus::Blocked,
                        Some(reason.into()),
                        MutationOrigin::Seed,
                    )?;
                }
            }
        }
        Ok(())
    }
}

const TASK_SELECT: &str = "SELECT id,project_id,title,description,status,priority,due_at,
 scheduled_at,started_at,completed_at,estimated_minutes,time_limit_minutes,pinned,blocked_reason,
 tags,created_at,updated_at FROM tasks";

fn get_or_create_device_id_with_connection(connection: &Connection) -> Result<String> {
    let now = timestamp(Utc::now());
    if let Some(device_id) = connection
        .query_row(
            "SELECT value FROM sync_state WHERE key=?1",
            [LOCAL_DEVICE_ID_KEY],
            |row| row.get(0),
        )
        .optional()?
    {
        connection.execute(
            "INSERT INTO sync_devices (device_id,name,created_at,last_seen_at)
             VALUES (?1,'Local device',?2,?2)
             ON CONFLICT(device_id) DO UPDATE SET last_seen_at=excluded.last_seen_at",
            params![device_id, now],
        )?;
        return Ok(device_id);
    }

    let device_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO sync_state (key,value) VALUES (?1,?2)",
        params![LOCAL_DEVICE_ID_KEY, device_id],
    )?;
    connection.execute(
        "INSERT INTO sync_devices (device_id,name,created_at,last_seen_at)
         VALUES (?1,'Local device',?2,?2)",
        params![device_id, now],
    )?;
    Ok(device_id)
}

fn get_sync_state_with_connection(connection: &Connection, key: &str) -> Result<Option<String>> {
    Ok(connection
        .query_row("SELECT value FROM sync_state WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

fn set_sync_state_with_connection(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO sync_state (key,value) VALUES (?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn set_optional_sync_state_with_connection(
    connection: &Connection,
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    if let Some(value) = value {
        set_sync_state_with_connection(connection, key, value)
    } else {
        connection.execute("DELETE FROM sync_state WHERE key=?1", [key])?;
        Ok(())
    }
}

fn get_sync_settings_with_connection(connection: &Connection) -> Result<(String, SyncSettings)> {
    let device_id = get_or_create_device_id_with_connection(connection)?;
    let enabled = match get_sync_state_with_connection(connection, SYNC_ENABLED_KEY)?.as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(value) => {
            return Err(CoreError::InvalidValue(format!(
                "invalid {SYNC_ENABLED_KEY}: {value}"
            )));
        }
    };
    let settings = SyncSettings {
        enabled,
        server_url: get_sync_state_with_connection(connection, SYNC_SERVER_URL_KEY)?,
        device_name: get_sync_state_with_connection(connection, SYNC_DEVICE_NAME_KEY)?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_DEVICE_NAME.into()),
        account_id: get_sync_state_with_connection(connection, SYNC_ACCOUNT_ID_KEY)?,
        user_id: get_sync_state_with_connection(connection, SYNC_USER_ID_KEY)?,
        device_token: get_sync_state_with_connection(connection, SYNC_DEVICE_TOKEN_KEY)?,
        last_successful_sync_at: parse_sync_state_time(
            SYNC_LAST_SUCCESSFUL_AT_KEY,
            get_sync_state_with_connection(connection, SYNC_LAST_SUCCESSFUL_AT_KEY)?,
        )?,
        last_attempted_sync_at: parse_sync_state_time(
            SYNC_LAST_ATTEMPTED_AT_KEY,
            get_sync_state_with_connection(connection, SYNC_LAST_ATTEMPTED_AT_KEY)?,
        )?,
    };
    validate_server_url(settings.server_url.as_deref())?;
    connection.execute(
        "UPDATE sync_devices SET name=?2 WHERE device_id=?1",
        params![device_id, settings.device_name],
    )?;
    Ok((device_id, settings))
}

fn get_sync_status_with_connection(connection: &Connection) -> Result<SyncStatus> {
    let (device_id, settings) = get_sync_settings_with_connection(connection)?;
    let unsynced_event_count = connection.query_row(
        "SELECT COUNT(*) FROM sync_events WHERE synced_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    let last_error = get_sync_state_with_connection(connection, SYNC_LAST_ERROR_KEY)?;
    let configured = settings.server_url.is_some();
    let state = if !settings.enabled {
        SyncConnectionState::Disabled
    } else if !configured {
        SyncConnectionState::NotConfigured
    } else if last_error.is_some() {
        SyncConnectionState::Error
    } else {
        SyncConnectionState::Ready
    };
    Ok(SyncStatus {
        state,
        enabled: settings.enabled,
        configured,
        server_url: settings.server_url,
        device_id,
        device_name: settings.device_name,
        unsynced_event_count,
        last_successful_sync_at: settings.last_successful_sync_at,
        last_attempted_sync_at: settings.last_attempted_sync_at,
        last_error,
    })
}

fn parse_sync_state_time(key: &str, value: Option<String>) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|error| CoreError::InvalidValue(format!("invalid {key}: {error}")))
        })
        .transpose()
}

fn remote_apply_dependency_rank(entity_type: SyncEntityType) -> u8 {
    match entity_type {
        SyncEntityType::Organization => 0,
        SyncEntityType::Project => 1,
        SyncEntityType::Task => 2,
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn validate_server_url(server_url: Option<&str>) -> Result<()> {
    if server_url
        .is_some_and(|value| !value.starts_with("http://") && !value.starts_with("https://"))
    {
        Err(CoreError::Validation(
            "sync server URL must start with http:// or https://".into(),
        ))
    } else {
        Ok(())
    }
}

fn append_sync_event_with_connection(
    connection: &Connection,
    entity_type: SyncEntityType,
    entity_id: &str,
    operation: SyncOperation,
    payload_json: serde_json::Value,
) -> Result<SyncEvent> {
    let device_id = get_or_create_device_id_with_connection(connection)?;
    let sequence = connection.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM sync_events WHERE device_id=?1",
        [&device_id],
        |row| row.get(0),
    )?;
    let event = SyncEvent {
        event_id: Uuid::new_v4().to_string(),
        device_id,
        actor_user_id: None,
        target_user_id: None,
        workspace_id: None,
        sequence,
        entity_type,
        entity_id: entity_id.to_owned(),
        operation,
        payload_json,
        created_at: Utc::now(),
        synced_at: None,
    };
    connection.execute(
        "INSERT INTO sync_events (
            event_id,device_id,actor_user_id,target_user_id,workspace_id,sequence,entity_type,
            entity_id,operation,payload_json,created_at,synced_at
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
            timestamp(event.created_at),
            Option::<String>::None,
        ],
    )?;
    Ok(event)
}

#[derive(serde::Deserialize)]
struct RemoteArchivePayload {
    id: String,
    archived_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct RemoteConflictOutcome {
    conflict_ids: Vec<String>,
    auto_resolved_conflict_count: usize,
}

struct RemoteConflictDecision {
    apply_remote: bool,
    conflict_kind: SyncConflictKind,
    policy_action: SyncConflictPolicyAction,
    resolution_status: SyncConflictResolutionStatus,
    local_snapshot_json: Option<serde_json::Value>,
    remote_snapshot_json: Option<serde_json::Value>,
}

struct PendingSyncConflict<'a> {
    event: &'a SyncEvent,
    local_device_id: &'a str,
    conflict_kind: SyncConflictKind,
    policy_action: SyncConflictPolicyAction,
    local_snapshot_json: Option<serde_json::Value>,
    remote_snapshot_json: Option<serde_json::Value>,
    resolution_status: SyncConflictResolutionStatus,
}

fn apply_remote_domain_change_with_policy(
    connection: &Connection,
    event: &SyncEvent,
    policy: &ConflictPolicy,
    local_device_id: &str,
) -> Result<RemoteConflictOutcome> {
    let decision = remote_conflict_decision(connection, event, policy)?;
    let mut outcome = RemoteConflictOutcome::default();
    if let Some(decision) = &decision {
        let conflict = record_sync_conflict_with_connection(
            connection,
            PendingSyncConflict {
                event,
                local_device_id,
                conflict_kind: decision.conflict_kind,
                policy_action: decision.policy_action,
                local_snapshot_json: decision.local_snapshot_json.clone(),
                remote_snapshot_json: decision.remote_snapshot_json.clone(),
                resolution_status: decision.resolution_status,
            },
        )?;
        if conflict.resolution_status == SyncConflictResolutionStatus::AutoResolved {
            outcome.auto_resolved_conflict_count += 1;
        }
        outcome.conflict_ids.push(conflict.conflict_id);
    }
    if decision
        .as_ref()
        .is_none_or(|decision| decision.apply_remote)
    {
        apply_remote_domain_change(connection, event)?;
    }
    Ok(outcome)
}

fn remote_conflict_decision(
    connection: &Connection,
    event: &SyncEvent,
    policy: &ConflictPolicy,
) -> Result<Option<RemoteConflictDecision>> {
    let local_snapshot_json =
        entity_snapshot_json(connection, event.entity_type, &event.entity_id)?;
    let remote_snapshot_json = remote_snapshot_json(event)?;
    let has_unsynced_local =
        has_unsynced_local_events_for_entity(connection, event.entity_type, &event.entity_id)?;

    if event.operation == SyncOperation::Archived {
        remote_archive(event)?;
        return Ok(has_unsynced_local.then_some(RemoteConflictDecision {
            apply_remote: true,
            conflict_kind: SyncConflictKind::ArchiveVsUpdate,
            policy_action: SyncConflictPolicyAction::AppliedRemote,
            resolution_status: SyncConflictResolutionStatus::Open,
            local_snapshot_json,
            remote_snapshot_json,
        }));
    }

    match (event.entity_type, event.operation) {
        (SyncEntityType::Organization, SyncOperation::Created | SyncOperation::Updated) => {
            let remote: Organization = remote_entity(event)?;
            ensure_event_entity_id(event, &remote.id)?;
            if local_snapshot_json
                .as_ref()
                .and_then(|value| serde_json::from_value::<Organization>(value.clone()).ok())
                .is_some_and(|local| local.archived_at.is_some() && remote.archived_at.is_none())
                && policy.organization.archive_vs_update
                    == crate::sync::ArchiveConflictStrategy::ArchiveWins
            {
                return Ok(Some(RemoteConflictDecision {
                    apply_remote: false,
                    conflict_kind: SyncConflictKind::ArchiveVsUpdate,
                    policy_action: SyncConflictPolicyAction::KeptLocal,
                    resolution_status: SyncConflictResolutionStatus::Open,
                    local_snapshot_json,
                    remote_snapshot_json,
                }));
            }
        }
        (SyncEntityType::Project, SyncOperation::Created | SyncOperation::Updated) => {
            let remote: Project = remote_entity(event)?;
            ensure_event_entity_id(event, &remote.id)?;
            if local_snapshot_json
                .as_ref()
                .and_then(|value| serde_json::from_value::<Project>(value.clone()).ok())
                .is_some_and(|local| {
                    (local.archived_at.is_some() || local.status == ProjectStatus::Archived)
                        && remote.archived_at.is_none()
                        && remote.status != ProjectStatus::Archived
                })
                && policy.project.archive_vs_update
                    == crate::sync::ArchiveConflictStrategy::ArchiveWins
            {
                return Ok(Some(RemoteConflictDecision {
                    apply_remote: false,
                    conflict_kind: SyncConflictKind::ArchiveVsUpdate,
                    policy_action: SyncConflictPolicyAction::KeptLocal,
                    resolution_status: SyncConflictResolutionStatus::Open,
                    local_snapshot_json,
                    remote_snapshot_json,
                }));
            }
        }
        (SyncEntityType::Task, SyncOperation::Created | SyncOperation::Updated) => {
            let remote: Task = remote_entity(event)?;
            ensure_event_entity_id(event, &remote.id)?;
            if event.operation == SyncOperation::Updated
                && policy.task.terminal_status_behavior
                    == crate::sync::TerminalStatusConflictStrategy::ProtectDoneCanceledArchived
                && local_snapshot_json
                    .as_ref()
                    .and_then(|value| serde_json::from_value::<Task>(value.clone()).ok())
                    .is_some_and(|local| {
                        is_terminal_task_status(local.status)
                            && !is_terminal_task_status(remote.status)
                    })
            {
                return Ok(Some(RemoteConflictDecision {
                    apply_remote: false,
                    conflict_kind: SyncConflictKind::TerminalStatusProtected,
                    policy_action: SyncConflictPolicyAction::KeptLocal,
                    resolution_status: SyncConflictResolutionStatus::Open,
                    local_snapshot_json,
                    remote_snapshot_json,
                }));
            }
        }
        (SyncEntityType::Task, SyncOperation::Transitioned) => {
            let remote: Task = remote_entity(event)?;
            ensure_event_entity_id(event, &remote.id)?;
        }
        _ => {}
    }

    Ok(has_unsynced_local.then_some(RemoteConflictDecision {
        apply_remote: match event.entity_type {
            SyncEntityType::Organization => {
                policy.organization.normal_update != crate::sync::FieldMergeStrategy::RecordConflict
            }
            SyncEntityType::Project => {
                policy.project.normal_update != crate::sync::FieldMergeStrategy::RecordConflict
            }
            SyncEntityType::Task => {
                policy.task.normal_update != crate::sync::FieldMergeStrategy::RecordConflict
            }
        },
        conflict_kind: SyncConflictKind::LocalUnsyncedChangeVsRemoteUpdate,
        policy_action: match event.entity_type {
            SyncEntityType::Organization
                if policy.organization.normal_update
                    == crate::sync::FieldMergeStrategy::RecordConflict =>
            {
                SyncConflictPolicyAction::RecordedOnly
            }
            SyncEntityType::Project
                if policy.project.normal_update
                    == crate::sync::FieldMergeStrategy::RecordConflict =>
            {
                SyncConflictPolicyAction::RecordedOnly
            }
            SyncEntityType::Task
                if policy.task.normal_update == crate::sync::FieldMergeStrategy::RecordConflict =>
            {
                SyncConflictPolicyAction::RecordedOnly
            }
            _ => SyncConflictPolicyAction::AppliedRemote,
        },
        resolution_status: SyncConflictResolutionStatus::Open,
        local_snapshot_json,
        remote_snapshot_json,
    }))
}

fn apply_remote_domain_change(connection: &Connection, event: &SyncEvent) -> Result<()> {
    match (event.entity_type, event.operation) {
        (SyncEntityType::Organization, SyncOperation::Created | SyncOperation::Updated) => {
            let organization: Organization = remote_entity(event)?;
            ensure_event_entity_id(event, &organization.id)?;
            upsert_organization_from_remote(connection, &organization)
        }
        (SyncEntityType::Organization, SyncOperation::Archived) => {
            let archive = remote_archive(event)?;
            changed(
                connection.execute(
                    "UPDATE organizations SET archived_at=?2,updated_at=?2 WHERE id=?1",
                    params![archive.id, timestamp(archive.archived_at)],
                )?,
                "organization",
            )
        }
        (SyncEntityType::Project, SyncOperation::Created | SyncOperation::Updated) => {
            let project: Project = remote_entity(event)?;
            ensure_event_entity_id(event, &project.id)?;
            upsert_project_from_remote(connection, &project)
        }
        (SyncEntityType::Project, SyncOperation::Archived) => {
            let archive = remote_archive(event)?;
            changed(
                connection.execute(
                    "UPDATE projects SET status='archived',archived_at=?2,updated_at=?2 WHERE id=?1",
                    params![archive.id, timestamp(archive.archived_at)],
                )?,
                "project",
            )
        }
        (
            SyncEntityType::Task,
            SyncOperation::Created | SyncOperation::Updated | SyncOperation::Transitioned,
        ) => {
            let task: Task = remote_entity(event)?;
            ensure_event_entity_id(event, &task.id)?;
            upsert_task_from_remote(connection, &task)
        }
        (SyncEntityType::Task, SyncOperation::Archived) => {
            let archive = remote_archive(event)?;
            changed(
                connection.execute(
                    "UPDATE tasks SET status='canceled',updated_at=?2 WHERE id=?1",
                    params![archive.id, timestamp(archive.archived_at)],
                )?,
                "task",
            )
        }
        (_, operation) => Err(CoreError::Validation(format!(
            "operation {operation} is not supported for remote {} events",
            event.entity_type
        ))),
    }
}

fn remote_entity<T: serde::de::DeserializeOwned>(event: &SyncEvent) -> Result<T> {
    let value = event.payload_json.get("entity").cloned().ok_or_else(|| {
        CoreError::Validation(format!(
            "remote {} event payload is missing entity",
            event.entity_type
        ))
    })?;
    serde_json::from_value(value).map_err(|error| {
        CoreError::Validation(format!(
            "invalid remote {} event entity: {error}",
            event.entity_type
        ))
    })
}

fn remote_archive(event: &SyncEvent) -> Result<RemoteArchivePayload> {
    let payload: RemoteArchivePayload = serde_json::from_value(event.payload_json.clone())
        .map_err(|error| {
            CoreError::Validation(format!(
                "invalid remote {} archive payload: {error}",
                event.entity_type
            ))
        })?;
    ensure_event_entity_id(event, &payload.id)?;
    Ok(payload)
}

fn ensure_event_entity_id(event: &SyncEvent, payload_id: &str) -> Result<()> {
    if event.entity_id == payload_id {
        Ok(())
    } else {
        Err(CoreError::Validation(format!(
            "remote event entity ID {} does not match payload ID {payload_id}",
            event.entity_id
        )))
    }
}

fn has_unsynced_local_events_for_entity(
    connection: &Connection,
    entity_type: SyncEntityType,
    entity_id: &str,
) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sync_events
            WHERE entity_type=?1 AND entity_id=?2 AND synced_at IS NULL
        )",
        params![entity_type.to_string(), entity_id],
        |row| row.get(0),
    )?)
}

fn entity_snapshot_json(
    connection: &Connection,
    entity_type: SyncEntityType,
    entity_id: &str,
) -> Result<Option<serde_json::Value>> {
    let value = match entity_type {
        SyncEntityType::Organization => {
            optional_snapshot(get_organization_with_connection(connection, entity_id))?
        }
        SyncEntityType::Project => {
            optional_snapshot(get_project_with_connection(connection, entity_id))?
        }
        SyncEntityType::Task => optional_snapshot(get_task_with_connection(connection, entity_id))?,
    };
    Ok(value)
}

fn optional_snapshot<T: serde::Serialize>(result: Result<T>) -> Result<Option<serde_json::Value>> {
    match result {
        Ok(value) => serde_json::to_value(value)
            .map(Some)
            .map_err(|error| CoreError::Validation(format!("invalid conflict snapshot: {error}"))),
        Err(CoreError::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn remote_snapshot_json(event: &SyncEvent) -> Result<Option<serde_json::Value>> {
    match event.operation {
        SyncOperation::Created | SyncOperation::Updated | SyncOperation::Transitioned => Ok(Some(
            event.payload_json.get("entity").cloned().ok_or_else(|| {
                CoreError::Validation(format!(
                    "remote {} event payload is missing entity",
                    event.entity_type
                ))
            })?,
        )),
        SyncOperation::Archived => {
            remote_archive(event)?;
            Ok(Some(event.payload_json.clone()))
        }
    }
}

fn record_sync_conflict_with_connection(
    connection: &Connection,
    conflict: PendingSyncConflict<'_>,
) -> Result<SyncConflict> {
    let conflict_id = Uuid::new_v4().to_string();
    let now = timestamp(Utc::now());
    connection.execute(
        "INSERT OR IGNORE INTO sync_conflicts (
            conflict_id,remote_event_id,local_device_id,entity_type,entity_id,conflict_kind,
            policy_action,local_snapshot_json,remote_snapshot_json,resolution_status,created_at,
            resolved_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL)",
        params![
            conflict_id,
            conflict.event.event_id,
            conflict.local_device_id,
            conflict.event.entity_type.to_string(),
            conflict.event.entity_id,
            conflict.conflict_kind.to_string(),
            conflict.policy_action.to_string(),
            conflict
                .local_snapshot_json
                .as_ref()
                .map(serde_json::Value::to_string),
            conflict
                .remote_snapshot_json
                .as_ref()
                .map(serde_json::Value::to_string),
            conflict.resolution_status.to_string(),
            now,
        ],
    )?;
    get_sync_conflict_by_remote_event_with_connection(connection, &conflict.event.event_id)
}

fn get_sync_conflict_by_remote_event_with_connection(
    connection: &Connection,
    remote_event_id: &str,
) -> Result<SyncConflict> {
    connection
        .query_row(
            "SELECT conflict_id,remote_event_id,local_device_id,entity_type,entity_id,
                    conflict_kind,policy_action,local_snapshot_json,remote_snapshot_json,
                    resolution_status,created_at,resolved_at
             FROM sync_conflicts WHERE remote_event_id=?1",
            [remote_event_id],
            map_sync_conflict,
        )
        .optional()?
        .ok_or(CoreError::NotFound("sync conflict"))
}

fn get_sync_conflict_with_connection(
    connection: &Connection,
    conflict_id: &str,
) -> Result<SyncConflict> {
    connection
        .query_row(
            "SELECT conflict_id,remote_event_id,local_device_id,entity_type,entity_id,
                    conflict_kind,policy_action,local_snapshot_json,remote_snapshot_json,
                    resolution_status,created_at,resolved_at
             FROM sync_conflicts WHERE conflict_id=?1",
            [conflict_id],
            map_sync_conflict,
        )
        .optional()?
        .ok_or(CoreError::NotFound("sync conflict"))
}

fn map_sync_conflict(row: &Row<'_>) -> rusqlite::Result<SyncConflict> {
    let local_snapshot_json: Option<String> = row.get(7)?;
    let remote_snapshot_json: Option<String> = row.get(8)?;
    Ok(SyncConflict {
        conflict_id: row.get(0)?,
        remote_event_id: row.get(1)?,
        local_device_id: row.get(2)?,
        entity_type: parse_enum(row.get::<_, String>(3)?)?,
        entity_id: row.get(4)?,
        conflict_kind: parse_enum(row.get::<_, String>(5)?)?,
        policy_action: parse_enum(row.get::<_, String>(6)?)?,
        local_snapshot_json: parse_optional_json(local_snapshot_json)?,
        remote_snapshot_json: parse_optional_json(remote_snapshot_json)?,
        resolution_status: parse_enum(row.get::<_, String>(9)?)?,
        created_at: parse_time(row.get(10)?)?,
        resolved_at: parse_optional_time(row.get(11)?)?,
    })
}

fn parse_optional_json(value: Option<String>) -> rusqlite::Result<Option<serde_json::Value>> {
    value
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

fn is_terminal_task_status(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Canceled)
}

fn entity_exists(connection: &Connection, table: &str, id: &str) -> Result<bool> {
    Ok(connection.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1)"),
        [id],
        |row| row.get(0),
    )?)
}

fn upsert_organization_from_remote(
    connection: &Connection,
    organization: &Organization,
) -> Result<()> {
    connection.execute(
        "INSERT INTO organizations (
            id,name,slug,description,color,icon,created_at,updated_at,archived_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(id) DO UPDATE SET
            name=excluded.name,slug=excluded.slug,description=excluded.description,
            color=excluded.color,icon=excluded.icon,created_at=excluded.created_at,
            updated_at=excluded.updated_at,archived_at=excluded.archived_at",
        params![
            organization.id,
            organization.name,
            organization.slug,
            organization.description,
            organization.color,
            organization.icon,
            timestamp(organization.created_at),
            timestamp(organization.updated_at),
            organization.archived_at.map(timestamp),
        ],
    )?;
    Ok(())
}

fn upsert_project_from_remote(connection: &Connection, project: &Project) -> Result<()> {
    if !entity_exists(connection, "organizations", &project.organization_id)? {
        return Err(CoreError::Validation(
            "cannot apply remote project event because organization is missing".into(),
        ));
    }
    connection.execute(
        "INSERT INTO projects (
            id,organization_id,name,slug,description,type,status,priority,deadline,repo_url,notes,
            created_at,updated_at,archived_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(id) DO UPDATE SET
            organization_id=excluded.organization_id,name=excluded.name,slug=excluded.slug,
            description=excluded.description,type=excluded.type,status=excluded.status,
            priority=excluded.priority,deadline=excluded.deadline,repo_url=excluded.repo_url,
            notes=excluded.notes,created_at=excluded.created_at,updated_at=excluded.updated_at,
            archived_at=excluded.archived_at",
        params![
            project.id,
            project.organization_id,
            project.name,
            project.slug,
            project.description,
            project.project_type.to_string(),
            project.status.to_string(),
            project.priority,
            project.deadline.map(timestamp),
            project.repo_url,
            project.notes,
            timestamp(project.created_at),
            timestamp(project.updated_at),
            project.archived_at.map(timestamp),
        ],
    )?;
    Ok(())
}

fn upsert_task_from_remote(connection: &Connection, task: &Task) -> Result<()> {
    if !entity_exists(connection, "projects", &task.project_id)? {
        return Err(CoreError::Validation(
            "cannot apply remote task event because project is missing".into(),
        ));
    }
    let tags = serde_json::to_string(&task.tags)
        .map_err(|error| CoreError::Validation(format!("invalid remote task tags: {error}")))?;
    connection.execute(
        "INSERT INTO tasks (
            id,project_id,title,description,status,priority,due_at,scheduled_at,started_at,
            completed_at,estimated_minutes,time_limit_minutes,pinned,blocked_reason,tags,
            created_at,updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
         ON CONFLICT(id) DO UPDATE SET
            project_id=excluded.project_id,title=excluded.title,description=excluded.description,
            status=excluded.status,priority=excluded.priority,due_at=excluded.due_at,
            scheduled_at=excluded.scheduled_at,started_at=excluded.started_at,
            completed_at=excluded.completed_at,estimated_minutes=excluded.estimated_minutes,
            time_limit_minutes=excluded.time_limit_minutes,pinned=excluded.pinned,
            blocked_reason=excluded.blocked_reason,tags=excluded.tags,
            created_at=excluded.created_at,updated_at=excluded.updated_at",
        params![
            task.id,
            task.project_id,
            task.title,
            task.description,
            task.status.to_string(),
            task.priority,
            task.due_at.map(timestamp),
            task.scheduled_at.map(timestamp),
            task.started_at.map(timestamp),
            task.completed_at.map(timestamp),
            task.estimated_minutes,
            task.time_limit_minutes,
            task.pinned,
            task.blocked_reason,
            tags,
            timestamp(task.created_at),
            timestamp(task.updated_at),
        ],
    )?;
    Ok(())
}

fn get_organization_with_connection(connection: &Connection, id: &str) -> Result<Organization> {
    connection
        .query_row(
            "SELECT id,name,slug,description,color,icon,created_at,updated_at,archived_at
             FROM organizations WHERE id=?1",
            [id],
            map_organization,
        )
        .optional()?
        .ok_or(CoreError::NotFound("organization"))
}

fn get_project_with_connection(connection: &Connection, id: &str) -> Result<Project> {
    connection
        .query_row(
            "SELECT id,organization_id,name,slug,description,type,status,priority,deadline,repo_url,
                    notes,created_at,updated_at,archived_at FROM projects WHERE id=?1",
            [id],
            map_project,
        )
        .optional()?
        .ok_or(CoreError::NotFound("project"))
}

fn get_task_with_connection(connection: &Connection, id: &str) -> Result<Task> {
    connection
        .query_row(&format!("{TASK_SELECT} WHERE id=?1"), [id], map_task)
        .optional()?
        .ok_or(CoreError::NotFound("task"))
}

fn save_task_with_connection(connection: &Connection, task: &Task) -> Result<()> {
    let status = task.status.to_string();
    let due_at = task.due_at.map(timestamp);
    let scheduled_at = task.scheduled_at.map(timestamp);
    let started_at = task.started_at.map(timestamp);
    let completed_at = task.completed_at.map(timestamp);
    let tags = serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".into());
    let created_at = timestamp(task.created_at);
    let updated_at = timestamp(task.updated_at);
    changed(
        connection.execute(
            "UPDATE tasks SET project_id=?2,title=?3,description=?4,status=?5,priority=?6,due_at=?7,
             scheduled_at=?8,started_at=?9,completed_at=?10,estimated_minutes=?11,
             time_limit_minutes=?12,pinned=?13,blocked_reason=?14,tags=?15,created_at=?16,
             updated_at=?17 WHERE id=?1",
            params![
                task.id,
                task.project_id,
                task.title,
                task.description,
                status,
                task.priority,
                due_at,
                scheduled_at,
                started_at,
                completed_at,
                task.estimated_minutes,
                task.time_limit_minutes,
                task.pinned,
                task.blocked_reason,
                tags,
                created_at,
                updated_at
            ],
        )?,
        "task",
    )
}

fn map_organization(row: &Row<'_>) -> rusqlite::Result<Organization> {
    Ok(Organization {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        description: row.get(3)?,
        color: row.get(4)?,
        icon: row.get(5)?,
        created_at: parse_time(row.get(6)?)?,
        updated_at: parse_time(row.get(7)?)?,
        archived_at: parse_optional_time(row.get(8)?)?,
    })
}

fn map_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        organization_id: row.get(1)?,
        name: row.get(2)?,
        slug: row.get(3)?,
        description: row.get(4)?,
        project_type: parse_enum(row.get::<_, String>(5)?)?,
        status: parse_enum(row.get::<_, String>(6)?)?,
        priority: row.get(7)?,
        deadline: parse_optional_time(row.get(8)?)?,
        repo_url: row.get(9)?,
        notes: row.get(10)?,
        created_at: parse_time(row.get(11)?)?,
        updated_at: parse_time(row.get(12)?)?,
        archived_at: parse_optional_time(row.get(13)?)?,
    })
}

fn map_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let tags: String = row.get(14)?;
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: parse_enum(row.get::<_, String>(4)?)?,
        priority: row.get(5)?,
        due_at: parse_optional_time(row.get(6)?)?,
        scheduled_at: parse_optional_time(row.get(7)?)?,
        started_at: parse_optional_time(row.get(8)?)?,
        completed_at: parse_optional_time(row.get(9)?)?,
        estimated_minutes: row.get(10)?,
        time_limit_minutes: row.get(11)?,
        pinned: row.get(12)?,
        blocked_reason: row.get(13)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        created_at: parse_time(row.get(15)?)?,
        updated_at: parse_time(row.get(16)?)?,
    })
}

fn map_sync_event(row: &Row<'_>) -> rusqlite::Result<SyncEvent> {
    let payload_json = row.get::<_, String>(9)?;
    Ok(SyncEvent {
        event_id: row.get(0)?,
        device_id: row.get(1)?,
        actor_user_id: row.get(2)?,
        target_user_id: row.get(3)?,
        workspace_id: row.get(4)?,
        sequence: row.get(5)?,
        entity_type: parse_enum(row.get::<_, String>(6)?)?,
        entity_id: row.get(7)?,
        operation: parse_enum(row.get::<_, String>(8)?)?,
        payload_json: serde_json::from_str(&payload_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        created_at: parse_time(row.get(10)?)?,
        synced_at: parse_optional_time(row.get(11)?)?,
    })
}

fn parse_time(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

fn parse_optional_time(value: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    value.map(parse_time).transpose()
}

fn parse_enum<T>(value: String) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value.parse::<T>().map_err(|error| {
        let error = std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string());
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in value.trim().to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            separator = false;
        } else if !slug.is_empty() && !separator {
            slug.push('-');
            separator = true;
        }
    }
    slug.trim_end_matches('-').to_owned()
}

fn require_name(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(CoreError::Validation("name cannot be empty".into()))
    } else {
        Ok(())
    }
}

fn validate_priority(value: i32) -> Result<()> {
    if (1..=5).contains(&value) {
        Ok(())
    } else {
        Err(CoreError::Validation(
            "priority must be between 1 and 5".into(),
        ))
    }
}

fn changed(rows: usize, entity: &'static str) -> Result<()> {
    if rows == 0 {
        Err(CoreError::NotFound(entity))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        ArchiveConflictStrategy, ConflictPolicy, FieldMergeStrategy, RemoteApplyStatus,
        RestoreConflictStrategy, StatusConflictStrategy, SyncConflictKind,
        SyncConflictPolicyAction, SyncConflictResolutionStatus, SyncConnectionState,
        SyncEntityType, SyncOperation, SyncSettingsPatch, TaskConflictPolicy,
        TerminalStatusConflictStrategy,
    };

    fn seeded_database() -> Database {
        let db = Database::in_memory().unwrap();
        db.seed().unwrap();
        db
    }

    fn remote_event(
        event_id: &str,
        entity_type: SyncEntityType,
        entity_id: &str,
        operation: SyncOperation,
        payload_json: serde_json::Value,
    ) -> SyncEvent {
        SyncEvent {
            event_id: event_id.into(),
            device_id: "remote-device".into(),
            actor_user_id: None,
            target_user_id: None,
            workspace_id: None,
            sequence: 1,
            entity_type,
            entity_id: entity_id.into(),
            operation,
            payload_json,
            created_at: Utc::now(),
            synced_at: None,
        }
    }

    fn remote_organization(id: &str) -> Organization {
        let now = Utc::now();
        Organization {
            id: id.into(),
            name: "Remote Organization".into(),
            slug: format!("remote-{id}"),
            description: None,
            color: None,
            icon: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        }
    }

    fn remote_project(id: &str, organization_id: &str) -> Project {
        let now = Utc::now();
        Project {
            id: id.into(),
            organization_id: organization_id.into(),
            name: "Remote Project".into(),
            slug: format!("remote-{id}"),
            description: None,
            project_type: ProjectType::Software,
            status: ProjectStatus::Active,
            priority: 3,
            deadline: None,
            repo_url: None,
            notes: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        }
    }

    fn remote_task(id: &str, project_id: &str) -> Task {
        let now = Utc::now();
        Task {
            id: id.into(),
            project_id: project_id.into(),
            title: "Remote Task".into(),
            description: None,
            status: TaskStatus::Inbox,
            priority: 3,
            due_at: None,
            scheduled_at: None,
            started_at: None,
            completed_at: None,
            estimated_minutes: None,
            time_limit_minutes: None,
            pinned: false,
            blocked_reason: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn migration_and_seed_are_idempotent() {
        let db = seeded_database();
        db.seed().unwrap();
        assert_eq!(db.list_organizations().unwrap().len(), 4);
        assert!(!db.list_projects().unwrap().is_empty());
        assert!(db.list_tasks().unwrap().len() >= 6);
    }

    #[test]
    fn migration_creates_sync_tables() {
        let db = Database::in_memory().unwrap();
        let connection = db.connection().unwrap();
        for table in [
            "sync_events",
            "sync_state",
            "sync_devices",
            "applied_remote_events",
            "sync_conflicts",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1
                    )",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing table {table}");
        }
        let sync_event_columns = connection
            .prepare("PRAGMA table_info(sync_events)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for column in ["actor_user_id", "target_user_id", "workspace_id"] {
            assert!(
                sync_event_columns.iter().any(|name| name == column),
                "missing column {column}"
            );
        }
    }

    #[test]
    fn default_conflict_policy_is_deterministic_and_serializable() {
        let policy = ConflictPolicy::default();

        assert_eq!(
            policy.organization.normal_update,
            FieldMergeStrategy::LastWriteWinsWholeEntity
        );
        assert_eq!(
            policy.organization.archive_vs_update,
            ArchiveConflictStrategy::ArchiveWins
        );
        assert_eq!(
            policy.organization.restore_behavior,
            RestoreConflictStrategy::ExplicitRestoreOnly
        );
        assert_eq!(
            policy.project.normal_update,
            FieldMergeStrategy::LastWriteWinsWholeEntity
        );
        assert_eq!(
            policy.task,
            TaskConflictPolicy {
                normal_update: FieldMergeStrategy::LastWriteWinsWholeEntity,
                status_update: StatusConflictStrategy::ServerOrderWins,
                terminal_status_behavior:
                    TerminalStatusConflictStrategy::ProtectDoneCanceledArchived,
                archive_vs_update: ArchiveConflictStrategy::ArchiveWins,
                restore_behavior: RestoreConflictStrategy::ExplicitRestoreOnly,
            }
        );

        let encoded = serde_json::to_string(&policy).unwrap();
        assert!(encoded.contains("last_write_wins_whole_entity"));
        assert_eq!(
            serde_json::from_str::<ConflictPolicy>(&encoded).unwrap(),
            policy
        );
    }

    #[test]
    fn remote_organization_apply_is_atomic_and_deduplicated() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-remote");
        let event = remote_event(
            "event-org",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Created,
            serde_json::json!({"entity": organization}),
        );

        assert_eq!(
            db.apply_remote_sync_event(&event).unwrap().status,
            RemoteApplyStatus::Applied
        );
        assert_eq!(
            db.apply_remote_sync_event(&event).unwrap().status,
            RemoteApplyStatus::AlreadyApplied
        );
        assert_eq!(db.list_organizations().unwrap().len(), 1);
        assert!(db.has_applied_remote_event(&event.event_id).unwrap());
        assert!(db.list_unsynced_events().unwrap().is_empty());

        let malformed = remote_event(
            "event-bad",
            SyncEntityType::Organization,
            "bad",
            SyncOperation::Created,
            serde_json::json!({}),
        );
        assert!(matches!(
            db.apply_remote_sync_event(&malformed),
            Err(CoreError::Validation(_))
        ));
        assert!(!db.has_applied_remote_event("event-bad").unwrap());
    }

    #[test]
    fn list_and_ignore_sync_conflicts() {
        let db = Database::in_memory().unwrap();
        let mut organization = remote_organization("org-conflict");
        db.apply_remote_sync_event(&remote_event(
            "event-create-conflict-org",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Created,
            serde_json::json!({"entity": organization.clone()}),
        ))
        .unwrap();
        db.update_organization(
            &organization.id,
            OrganizationPatch {
                description: Some(Some("local unsynced".into())),
                ..Default::default()
            },
        )
        .unwrap();
        organization.name = "Remote conflict update".into();
        db.apply_remote_sync_event(&remote_event(
            "event-conflict-org",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Updated,
            serde_json::json!({"entity": organization}),
        ))
        .unwrap();

        let conflicts = db.list_open_sync_conflicts().unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            conflicts[0].conflict_kind,
            SyncConflictKind::LocalUnsyncedChangeVsRemoteUpdate
        );
        assert_eq!(
            conflicts[0].policy_action,
            SyncConflictPolicyAction::AppliedRemote
        );
        assert_eq!(
            conflicts[0].resolution_status,
            SyncConflictResolutionStatus::Open
        );
        assert!(conflicts[0].local_snapshot_json.is_some());
        assert!(conflicts[0].remote_snapshot_json.is_some());

        let ignored = db
            .mark_sync_conflict_ignored(&conflicts[0].conflict_id)
            .unwrap();
        assert_eq!(
            ignored.resolution_status,
            SyncConflictResolutionStatus::Ignored
        );
        assert!(ignored.resolved_at.is_some());
        assert!(db.list_open_sync_conflicts().unwrap().is_empty());
    }

    #[test]
    fn remote_local_echo_is_skipped_without_mutation() {
        let db = Database::in_memory().unwrap();
        let mut event = remote_event(
            "event-echo",
            SyncEntityType::Organization,
            "echo-org",
            SyncOperation::Created,
            serde_json::json!({"entity": remote_organization("echo-org")}),
        );
        event.device_id = db.get_or_create_device_id().unwrap();

        assert_eq!(
            db.apply_remote_sync_event(&event).unwrap().status,
            RemoteApplyStatus::SkippedLocalEcho
        );
        assert!(db.list_organizations().unwrap().is_empty());
        assert!(!db.has_applied_remote_event(&event.event_id).unwrap());
    }

    #[test]
    fn remote_dependencies_are_required_and_ordered_batches_apply() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-1");
        let project = remote_project("project-1", &organization.id);
        let task = remote_task("task-1", &project.id);
        let organization_event = remote_event(
            "event-1",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Created,
            serde_json::json!({"entity": organization}),
        );
        let project_event = remote_event(
            "event-2",
            SyncEntityType::Project,
            &project.id,
            SyncOperation::Created,
            serde_json::json!({"entity": project}),
        );
        let task_event = remote_event(
            "event-3",
            SyncEntityType::Task,
            &task.id,
            SyncOperation::Created,
            serde_json::json!({"entity": task}),
        );

        assert!(matches!(
            db.apply_remote_sync_event(&project_event),
            Err(CoreError::Validation(_))
        ));
        assert!(!db.has_applied_remote_event("event-2").unwrap());
        assert!(matches!(
            db.apply_remote_sync_event(&task_event),
            Err(CoreError::Validation(_))
        ));

        let result = db
            .apply_remote_sync_events(&[organization_event, project_event, task_event])
            .unwrap();
        assert_eq!(result.applied_count, 3);
        assert_eq!(db.get_task("task-1").unwrap().title, "Remote Task");
        assert!(db.list_unsynced_events().unwrap().is_empty());
    }

    #[test]
    fn remote_batches_apply_dependency_order_even_when_pulled_out_of_order() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-1");
        let project = remote_project("project-1", &organization.id);
        let task = remote_task("task-1", &project.id);
        let organization_event = remote_event(
            "event-1",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Created,
            serde_json::json!({"entity": organization}),
        );
        let project_event = remote_event(
            "event-2",
            SyncEntityType::Project,
            &project.id,
            SyncOperation::Created,
            serde_json::json!({"entity": project}),
        );
        let task_event = remote_event(
            "event-3",
            SyncEntityType::Task,
            &task.id,
            SyncOperation::Created,
            serde_json::json!({"entity": task}),
        );

        let result = db
            .apply_remote_sync_events(&[task_event, project_event, organization_event])
            .unwrap();

        assert_eq!(result.applied_count, 3);
        assert_eq!(db.get_task("task-1").unwrap().title, "Remote Task");
        assert!(db.list_unsynced_events().unwrap().is_empty());
    }

    #[test]
    fn remote_task_update_and_archives_do_not_log_local_events() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-1");
        let project = remote_project("project-1", &organization.id);
        let task = remote_task("task-1", &project.id);
        db.apply_remote_sync_events(&[
            remote_event(
                "event-1",
                SyncEntityType::Organization,
                &organization.id,
                SyncOperation::Created,
                serde_json::json!({"entity": organization}),
            ),
            remote_event(
                "event-2",
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Created,
                serde_json::json!({"entity": project}),
            ),
            remote_event(
                "event-3",
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Created,
                serde_json::json!({"entity": task}),
            ),
        ])
        .unwrap();

        let mut updated = db.get_task("task-1").unwrap();
        updated.title = "Updated Remotely".into();
        updated.status = TaskStatus::InProgress;
        updated.updated_at = Utc::now();
        db.apply_remote_sync_event(&remote_event(
            "event-4",
            SyncEntityType::Task,
            &updated.id,
            SyncOperation::Transitioned,
            serde_json::json!({"entity": updated}),
        ))
        .unwrap();
        assert_eq!(db.get_task("task-1").unwrap().title, "Updated Remotely");

        let archived_at = Utc::now();
        db.apply_remote_sync_events(&[
            remote_event(
                "event-5",
                SyncEntityType::Project,
                "project-1",
                SyncOperation::Archived,
                serde_json::json!({"id": "project-1", "archived_at": archived_at}),
            ),
            remote_event(
                "event-6",
                SyncEntityType::Organization,
                "org-1",
                SyncOperation::Archived,
                serde_json::json!({"id": "org-1", "archived_at": archived_at}),
            ),
        ])
        .unwrap();
        assert_eq!(
            db.get_project("project-1").unwrap().status,
            ProjectStatus::Archived
        );
        assert!(db.get_organization("org-1").unwrap().archived_at.is_some());
        assert!(db.list_unsynced_events().unwrap().is_empty());
    }

    #[test]
    fn local_unsynced_task_update_records_conflict_and_remote_apply_succeeds() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-1");
        let project = remote_project("project-1", &organization.id);
        let task = remote_task("task-1", &project.id);
        db.apply_remote_sync_events(&[
            remote_event(
                "event-conflict-org-1",
                SyncEntityType::Organization,
                &organization.id,
                SyncOperation::Created,
                serde_json::json!({"entity": organization}),
            ),
            remote_event(
                "event-conflict-project-1",
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Created,
                serde_json::json!({"entity": project}),
            ),
            remote_event(
                "event-conflict-task-1",
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Created,
                serde_json::json!({"entity": task}),
            ),
        ])
        .unwrap();

        db.update_task(
            "task-1",
            TaskPatch {
                title: Some("Local unsynced title".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut remote = db.get_task("task-1").unwrap();
        remote.title = "Remote title".into();
        let result = db
            .apply_remote_sync_event(&remote_event(
                "event-conflict-task-update",
                SyncEntityType::Task,
                "task-1",
                SyncOperation::Updated,
                serde_json::json!({"entity": remote}),
            ))
            .unwrap();

        assert_eq!(result.status, RemoteApplyStatus::Applied);
        assert_eq!(result.conflict_ids.len(), 1);
        assert_eq!(db.get_task("task-1").unwrap().title, "Remote title");
        assert_eq!(db.list_open_sync_conflicts().unwrap().len(), 1);
        assert!(
            db.has_applied_remote_event("event-conflict-task-update")
                .unwrap()
        );
        assert_eq!(db.list_unsynced_events().unwrap().len(), 1);
    }

    #[test]
    fn local_archived_organization_is_not_restored_by_remote_normal_update() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-archive");
        db.apply_remote_sync_event(&remote_event(
            "event-archive-org-create",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Created,
            serde_json::json!({"entity": organization.clone()}),
        ))
        .unwrap();
        db.archive_organization(&organization.id).unwrap();

        let mut remote = organization;
        remote.name = "Remote normal update".into();
        remote.archived_at = None;
        let result = db
            .apply_remote_sync_event(&remote_event(
                "event-archive-org-update",
                SyncEntityType::Organization,
                &remote.id,
                SyncOperation::Updated,
                serde_json::json!({"entity": remote}),
            ))
            .unwrap();

        assert_eq!(result.conflict_ids.len(), 1);
        assert!(
            db.get_organization("org-archive")
                .unwrap()
                .archived_at
                .is_some()
        );
        let conflict = db.list_open_sync_conflicts().unwrap().remove(0);
        assert_eq!(conflict.conflict_kind, SyncConflictKind::ArchiveVsUpdate);
        assert_eq!(conflict.policy_action, SyncConflictPolicyAction::KeptLocal);
        assert!(
            db.has_applied_remote_event("event-archive-org-update")
                .unwrap()
        );
    }

    #[test]
    fn remote_archive_wins_over_local_unsynced_project_update() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-archive-win");
        let project = remote_project("project-archive-win", &organization.id);
        db.apply_remote_sync_events(&[
            remote_event(
                "event-archive-win-org",
                SyncEntityType::Organization,
                &organization.id,
                SyncOperation::Created,
                serde_json::json!({"entity": organization}),
            ),
            remote_event(
                "event-archive-win-project",
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Created,
                serde_json::json!({"entity": project.clone()}),
            ),
        ])
        .unwrap();
        db.update_project(
            &project.id,
            ProjectPatch {
                name: Some("Local project update".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let archived_at = Utc::now();
        let result = db
            .apply_remote_sync_event(&remote_event(
                "event-archive-win-project-archive",
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Archived,
                serde_json::json!({"id": project.id, "archived_at": archived_at}),
            ))
            .unwrap();

        assert_eq!(result.conflict_ids.len(), 1);
        assert_eq!(
            db.get_project("project-archive-win").unwrap().status,
            ProjectStatus::Archived
        );
        let conflict = db.list_open_sync_conflicts().unwrap().remove(0);
        assert_eq!(conflict.conflict_kind, SyncConflictKind::ArchiveVsUpdate);
        assert_eq!(
            conflict.policy_action,
            SyncConflictPolicyAction::AppliedRemote
        );
    }

    #[test]
    fn local_terminal_task_is_not_reverted_by_remote_normal_update() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-terminal");
        let project = remote_project("project-terminal", &organization.id);
        let task = remote_task("task-terminal", &project.id);
        db.apply_remote_sync_events(&[
            remote_event(
                "event-terminal-org",
                SyncEntityType::Organization,
                &organization.id,
                SyncOperation::Created,
                serde_json::json!({"entity": organization}),
            ),
            remote_event(
                "event-terminal-project",
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Created,
                serde_json::json!({"entity": project}),
            ),
            remote_event(
                "event-terminal-task",
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Created,
                serde_json::json!({"entity": task}),
            ),
        ])
        .unwrap();
        db.transition_task("task-terminal", TaskStatus::Done, None)
            .unwrap();

        let mut remote = db.get_task("task-terminal").unwrap();
        remote.status = TaskStatus::Ready;
        remote.completed_at = None;
        remote.title = "Stale remote update".into();
        let result = db
            .apply_remote_sync_event(&remote_event(
                "event-terminal-task-update",
                SyncEntityType::Task,
                "task-terminal",
                SyncOperation::Updated,
                serde_json::json!({"entity": remote}),
            ))
            .unwrap();

        assert_eq!(result.conflict_ids.len(), 1);
        let task = db.get_task("task-terminal").unwrap();
        assert_eq!(task.status, TaskStatus::Done);
        assert_ne!(task.title, "Stale remote update");
        let conflict = db.list_open_sync_conflicts().unwrap().remove(0);
        assert_eq!(
            conflict.conflict_kind,
            SyncConflictKind::TerminalStatusProtected
        );
        assert_eq!(conflict.policy_action, SyncConflictPolicyAction::KeptLocal);
    }

    #[test]
    fn explicit_remote_task_transition_uses_server_order_over_terminal_local_state() {
        let db = Database::in_memory().unwrap();
        let organization = remote_organization("org-transition");
        let project = remote_project("project-transition", &organization.id);
        let task = remote_task("task-transition", &project.id);
        db.apply_remote_sync_events(&[
            remote_event(
                "event-transition-org",
                SyncEntityType::Organization,
                &organization.id,
                SyncOperation::Created,
                serde_json::json!({"entity": organization}),
            ),
            remote_event(
                "event-transition-project",
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Created,
                serde_json::json!({"entity": project}),
            ),
            remote_event(
                "event-transition-task",
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Created,
                serde_json::json!({"entity": task}),
            ),
        ])
        .unwrap();
        db.transition_task("task-transition", TaskStatus::Done, None)
            .unwrap();

        let mut remote = db.get_task("task-transition").unwrap();
        remote.status = TaskStatus::Ready;
        remote.completed_at = None;
        let result = db
            .apply_remote_sync_event(&remote_event(
                "event-transition-task-ready",
                SyncEntityType::Task,
                "task-transition",
                SyncOperation::Transitioned,
                serde_json::json!({"entity": remote}),
            ))
            .unwrap();

        assert_eq!(result.conflict_ids.len(), 1);
        assert_eq!(
            db.get_task("task-transition").unwrap().status,
            TaskStatus::Ready
        );
        let conflict = db.list_open_sync_conflicts().unwrap().remove(0);
        assert_eq!(
            conflict.conflict_kind,
            SyncConflictKind::LocalUnsyncedChangeVsRemoteUpdate
        );
        assert_eq!(
            conflict.policy_action,
            SyncConflictPolicyAction::AppliedRemote
        );
    }

    #[test]
    fn duplicate_remote_conflict_record_is_idempotent() {
        let db = Database::in_memory().unwrap();
        let mut organization = remote_organization("org-idempotent-conflict");
        db.apply_remote_sync_event(&remote_event(
            "event-idempotent-conflict-create",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Created,
            serde_json::json!({"entity": organization.clone()}),
        ))
        .unwrap();
        db.update_organization(
            &organization.id,
            OrganizationPatch {
                name: Some("Local unsynced name".into()),
                ..Default::default()
            },
        )
        .unwrap();
        organization.name = "Remote idempotent name".into();
        let event = remote_event(
            "event-idempotent-conflict-update",
            SyncEntityType::Organization,
            &organization.id,
            SyncOperation::Updated,
            serde_json::json!({"entity": organization}),
        );

        let first = db.apply_remote_sync_event(&event).unwrap();
        let second = db.apply_remote_sync_event(&event).unwrap();

        assert_eq!(first.conflict_ids.len(), 1);
        assert!(second.conflict_ids.is_empty());
        assert_eq!(db.list_sync_conflicts().unwrap().len(), 1);
    }

    #[test]
    fn migration_upgrades_existing_sync_events_with_ownership_columns() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sync_events (
                    event_id TEXT PRIMARY KEY NOT NULL,
                    device_id TEXT NOT NULL,
                    sequence INTEGER NOT NULL,
                    entity_type TEXT NOT NULL,
                    entity_id TEXT NOT NULL,
                    operation TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    synced_at TEXT,
                    UNIQUE(device_id, sequence)
                );",
            )
            .unwrap();
        let db = Database {
            connection: Arc::new(Mutex::new(connection)),
        };

        db.migrate().unwrap();

        let connection = db.connection().unwrap();
        let columns = connection
            .prepare("PRAGMA table_info(sync_events)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for column in ["actor_user_id", "target_user_id", "workspace_id"] {
            assert!(columns.iter().any(|name| name == column));
        }
    }

    #[test]
    fn device_id_is_stable() {
        let db = Database::in_memory().unwrap();
        let first = db.get_or_create_device_id().unwrap();
        let second = db.get_or_create_device_id().unwrap();
        assert_eq!(first, second);
        assert!(Uuid::parse_str(&first).is_ok());
    }

    #[test]
    fn device_id_registers_and_updates_local_device() {
        let db = Database::in_memory().unwrap();
        let device_id = db.get_or_create_device_id().unwrap();
        {
            let connection = db.connection().unwrap();
            connection
                .execute(
                    "UPDATE sync_devices SET last_seen_at='2000-01-01T00:00:00Z'
                     WHERE device_id=?1",
                    [&device_id],
                )
                .unwrap();
        }
        assert_eq!(db.get_or_create_device_id().unwrap(), device_id);

        let connection = db.connection().unwrap();
        let state_device_id: String = connection
            .query_row(
                "SELECT value FROM sync_state WHERE key=?1",
                [LOCAL_DEVICE_ID_KEY],
                |row| row.get(0),
            )
            .unwrap();
        let (name, created_at, last_seen_at): (String, String, Option<String>) = connection
            .query_row(
                "SELECT name,created_at,last_seen_at FROM sync_devices WHERE device_id=?1",
                [&device_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state_device_id, device_id);
        assert_eq!(name, "Local device");
        assert!(DateTime::parse_from_rfc3339(&created_at).is_ok());
        assert_ne!(last_seen_at.as_deref(), Some("2000-01-01T00:00:00Z"));
    }

    #[test]
    fn sync_event_failure_rolls_back_domain_mutation() {
        let db = Database::in_memory().unwrap();
        db.connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_sync_events
                 BEFORE INSERT ON sync_events
                 BEGIN
                   SELECT RAISE(FAIL, 'sync event rejected');
                 END;",
            )
            .unwrap();

        let result = db.create_organization(NewOrganization {
            name: "Rolled Back".into(),
            slug: Some("rolled-back".into()),
            description: None,
            color: None,
            icon: None,
        });

        assert!(matches!(result, Err(CoreError::Database(_))));
        let exists: bool = db
            .connection()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM organizations WHERE slug='rolled-back')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists);
    }

    #[test]
    fn every_local_public_mutation_appends_one_event() {
        let db = Database::in_memory().unwrap();
        let event_count = || db.list_unsynced_events().unwrap().len();

        let organization = db
            .create_organization(NewOrganization {
                name: "Event Organization".into(),
                slug: None,
                description: None,
                color: None,
                icon: None,
            })
            .unwrap();
        assert_eq!(event_count(), 1);
        db.update_organization(
            &organization.id,
            OrganizationPatch {
                description: Some(Some("Updated".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(event_count(), 2);

        let project = db
            .create_project(NewProject {
                organization_id: organization.id.clone(),
                name: "Event Project".into(),
                slug: None,
                description: None,
                project_type: ProjectType::Software,
                status: ProjectStatus::Active,
                priority: 3,
                deadline: None,
                repo_url: None,
                notes: None,
            })
            .unwrap();
        assert_eq!(event_count(), 3);
        db.update_project(
            &project.id,
            ProjectPatch {
                notes: Some(Some("Updated".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(event_count(), 4);

        let task = db
            .create_task(NewTask {
                project_id: project.id.clone(),
                title: "Event Task".into(),
                description: None,
                status: TaskStatus::Inbox,
                priority: 3,
                due_at: None,
                scheduled_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                tags: Vec::new(),
            })
            .unwrap();
        assert_eq!(event_count(), 5);
        db.update_task(
            &task.id,
            TaskPatch {
                priority: Some(4),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(event_count(), 6);
        db.transition_task(&task.id, TaskStatus::Ready, None)
            .unwrap();
        assert_eq!(event_count(), 7);

        db.archive_project(&project.id).unwrap();
        assert_eq!(event_count(), 8);
        db.archive_organization(&organization.id).unwrap();
        assert_eq!(event_count(), 9);
    }

    #[test]
    fn task_mutations_append_and_sync_events() {
        let db = Database::in_memory().unwrap();
        let organization = db
            .create_organization(NewOrganization {
                name: "Test Organization".into(),
                slug: None,
                description: None,
                color: None,
                icon: None,
            })
            .unwrap();
        let project = db
            .create_project(NewProject {
                organization_id: organization.id,
                name: "Test Project".into(),
                slug: None,
                description: None,
                project_type: ProjectType::Software,
                status: ProjectStatus::Active,
                priority: 3,
                deadline: None,
                repo_url: None,
                notes: None,
            })
            .unwrap();
        let setup_event_ids = db
            .list_unsynced_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>();
        db.mark_sync_events_synced(&setup_event_ids).unwrap();
        let task = db
            .create_task(NewTask {
                project_id: project.id,
                title: "Test Task".into(),
                description: None,
                status: TaskStatus::Inbox,
                priority: 3,
                due_at: None,
                scheduled_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                tags: Vec::new(),
            })
            .unwrap();

        let events = db.list_unsynced_events().unwrap();
        assert_eq!(events.len(), 1);
        let created = events.first().unwrap();
        assert_eq!(created.entity_type, SyncEntityType::Task);
        assert_eq!(created.operation, SyncOperation::Created);
        assert_eq!(created.entity_id, task.id);
        assert_eq!(created.actor_user_id, None);
        assert_eq!(created.target_user_id, None);
        assert_eq!(created.workspace_id, None);

        db.update_task(
            &task.id,
            TaskPatch {
                title: Some("Updated Task".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let events = db.list_unsynced_events().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events.last().unwrap().operation, SyncOperation::Updated);

        let event_ids = events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        db.mark_sync_events_synced(&event_ids).unwrap();
        assert!(db.list_unsynced_events().unwrap().is_empty());
    }

    #[test]
    fn seed_does_not_append_sync_events() {
        let db = Database::in_memory().unwrap();
        db.seed().unwrap();
        assert!(db.list_unsynced_events().unwrap().is_empty());
        db.seed().unwrap();
        assert!(db.list_unsynced_events().unwrap().is_empty());
    }

    #[test]
    fn default_sync_settings_and_status_are_disabled() {
        let db = Database::in_memory().unwrap();

        let settings = db.get_sync_settings().unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.server_url, None);
        assert_eq!(settings.device_name, "Local device");
        assert_eq!(settings.account_id, None);
        assert_eq!(settings.user_id, None);
        assert_eq!(settings.device_token, None);
        assert_eq!(settings.last_successful_sync_at, None);
        assert_eq!(settings.last_attempted_sync_at, None);

        let status = db.get_sync_status().unwrap();
        assert_eq!(status.state, SyncConnectionState::Disabled);
        assert!(!status.enabled);
        assert!(!status.configured);
        assert_eq!(status.unsynced_event_count, 0);
    }

    #[test]
    fn sync_status_moves_from_not_configured_to_ready() {
        let db = Database::in_memory().unwrap();

        db.update_sync_settings(SyncSettingsPatch {
            enabled: Some(true),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            db.get_sync_status().unwrap().state,
            SyncConnectionState::NotConfigured
        );

        db.update_sync_settings(SyncSettingsPatch {
            server_url: Some(Some("http://127.0.0.1:8787".into())),
            ..Default::default()
        })
        .unwrap();
        let status = db.get_sync_status().unwrap();
        assert_eq!(status.state, SyncConnectionState::Ready);
        assert!(status.configured);
        assert_eq!(status.server_url.as_deref(), Some("http://127.0.0.1:8787"));
    }

    #[test]
    fn sync_settings_validate_urls_and_normalize_empty_values() {
        let db = Database::in_memory().unwrap();

        let error = db
            .update_sync_settings(SyncSettingsPatch {
                server_url: Some(Some("ftp://example.com".into())),
                ..Default::default()
            })
            .unwrap_err();
        assert!(matches!(error, CoreError::Validation(_)));

        let settings = db
            .update_sync_settings(SyncSettingsPatch {
                server_url: Some(Some("   ".into())),
                account_id: Some(Some(" ".into())),
                user_id: Some(Some("\t".into())),
                device_token: Some(Some("\n".into())),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(settings.server_url, None);
        assert_eq!(settings.account_id, None);
        assert_eq!(settings.user_id, None);
        assert_eq!(settings.device_token, None);
    }

    #[test]
    fn sync_device_name_update_persists_to_device_metadata() {
        let db = Database::in_memory().unwrap();

        let settings = db
            .update_sync_settings(SyncSettingsPatch {
                device_name: Some("Office Desktop".into()),
                ..Default::default()
            })
            .unwrap();
        let device_id = db.get_or_create_device_id().unwrap();
        let stored_name: String = db
            .connection()
            .unwrap()
            .query_row(
                "SELECT name FROM sync_devices WHERE device_id=?1",
                [&device_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(settings.device_name, "Office Desktop");
        assert_eq!(
            db.get_sync_settings().unwrap().device_name,
            "Office Desktop"
        );
        assert_eq!(stored_name, "Office Desktop");
    }

    #[test]
    fn sync_status_includes_unsynced_event_count() {
        let db = Database::in_memory().unwrap();
        db.append_sync_event(
            SyncEntityType::Task,
            "task-1",
            SyncOperation::Created,
            serde_json::json!({"entity": {"id": "task-1"}}),
        )
        .unwrap();

        assert_eq!(db.get_sync_status().unwrap().unsynced_event_count, 1);
    }

    #[test]
    fn sync_attempt_success_and_error_update_status() {
        let db = Database::in_memory().unwrap();
        db.update_sync_settings(SyncSettingsPatch {
            enabled: Some(true),
            server_url: Some(Some("https://sync.example.com".into())),
            ..Default::default()
        })
        .unwrap();

        let attempted = db.record_sync_attempt_started().unwrap();
        assert!(attempted.last_attempted_sync_at.is_some());

        let failed = db.record_sync_error("connection refused").unwrap();
        assert_eq!(failed.state, SyncConnectionState::Error);
        assert_eq!(failed.last_error.as_deref(), Some("connection refused"));

        let succeeded = db.record_sync_success().unwrap();
        assert_eq!(succeeded.state, SyncConnectionState::Ready);
        assert!(succeeded.last_successful_sync_at.is_some());
        assert_eq!(succeeded.last_error, None);
    }

    #[test]
    fn clearing_sync_error_preserves_ready_configuration() {
        let db = Database::in_memory().unwrap();
        db.update_sync_settings(SyncSettingsPatch {
            enabled: Some(true),
            server_url: Some(Some("https://sync.example.com".into())),
            ..Default::default()
        })
        .unwrap();
        db.record_sync_error("temporary failure").unwrap();

        let status = db.clear_sync_error().unwrap();

        assert_eq!(status.state, SyncConnectionState::Ready);
        assert_eq!(status.last_error, None);
        assert!(status.enabled);
        assert!(status.configured);
    }

    #[test]
    fn start_task_persists_in_progress_and_started_at() {
        let db = seeded_database();
        let task = db
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|task| task.status == TaskStatus::Inbox)
            .unwrap();
        db.transition_task(&task.id, TaskStatus::InProgress, None)
            .unwrap();
        let persisted = db.get_task(&task.id).unwrap();
        assert_eq!(persisted.status, TaskStatus::InProgress);
        assert!(persisted.started_at.is_some());
        assert!(
            db.board_state()
                .unwrap()
                .now
                .iter()
                .any(|item| item.context.task.id == task.id)
        );
    }

    #[test]
    fn complete_task_persists_and_appears_in_done_today() {
        let db = seeded_database();
        let task = db
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|task| task.status == TaskStatus::Inbox)
            .unwrap();
        db.transition_task(&task.id, TaskStatus::Done, None)
            .unwrap();
        let persisted = db.get_task(&task.id).unwrap();
        assert_eq!(persisted.status, TaskStatus::Done);
        assert!(persisted.completed_at.is_some());
        assert!(
            db.board_state()
                .unwrap()
                .done_today
                .iter()
                .any(|item| item.context.task.id == task.id)
        );
    }

    #[test]
    fn organization_updates_are_persisted() {
        let db = seeded_database();
        let organization = db.list_organizations().unwrap().remove(0);
        db.update_organization(
            &organization.id,
            OrganizationPatch {
                name: Some("Updated Organization".into()),
                description: Some(Some("Updated description".into())),
                color: Some(Some("#123456".into())),
                icon: Some(Some("UP".into())),
                ..Default::default()
            },
        )
        .unwrap();
        let persisted = db.get_organization(&organization.id).unwrap();
        assert_eq!(persisted.name, "Updated Organization");
        assert_eq!(
            persisted.description.as_deref(),
            Some("Updated description")
        );
        assert_eq!(persisted.color.as_deref(), Some("#123456"));
        assert_eq!(persisted.icon.as_deref(), Some("UP"));
    }

    #[test]
    fn seeded_active_tasks_produce_a_non_empty_board() {
        let board = seeded_database().board_state().unwrap();
        let active_count = board.now.len()
            + board.next_up.len()
            + board.due_soon.len()
            + board.waiting_blocked.len()
            + board.later_today.len()
            + board.overdue.len();
        assert!(active_count > 0);
    }
}
