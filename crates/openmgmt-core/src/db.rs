use crate::{
    board::build_board,
    models::{
        ActiveTimerInfo, BoardState, NewOrganization, NewProject, NewSavedTaskView, NewTask,
        Organization, OrganizationPatch, Project, ProjectPatch, ProjectStatus, ProjectType,
        SavedTaskView, SavedTaskViewPatch, ScoringSettings, ScoringSettingsPatch, Task,
        TaskContext, TaskPatch, TaskQueryFilter, TaskSort, TaskSortField, TaskStatus,
        TaskTimerSession, TaskWithContext,
    },
    scoring::{ScoringWeights, score_task},
    sync::{
        DEFAULT_DEVICE_NAME, LOCAL_DEVICE_ID_KEY, RemoteApplyBatchResult, RemoteApplyResult,
        RemoteApplyStatus, SYNC_ACCOUNT_ID_KEY, SYNC_DEVICE_NAME_KEY, SYNC_DEVICE_TOKEN_KEY,
        SYNC_ENABLED_KEY, SYNC_LAST_ATTEMPTED_AT_KEY, SYNC_LAST_ERROR_KEY,
        SYNC_LAST_SUCCESSFUL_AT_KEY, SYNC_SERVER_URL_KEY, SYNC_USER_ID_KEY, SyncConnectionState,
        SyncEntityType, SyncEvent, SyncOperation, SyncSettings, SyncSettingsPatch, SyncStatus,
    },
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::{
    cmp::Ordering,
    collections::HashMap,
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
            CREATE TABLE IF NOT EXISTS task_timer_sessions (
              id TEXT PRIMARY KEY NOT NULL,
              task_id TEXT NOT NULL REFERENCES tasks(id),
              started_at TEXT NOT NULL,
              paused_at TEXT,
              resumed_at TEXT,
              stopped_at TEXT,
              completed_at TEXT,
              duration_seconds INTEGER,
              note TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS task_timer_sessions_task_idx
              ON task_timer_sessions(task_id, started_at);
            CREATE INDEX IF NOT EXISTS task_timer_sessions_active_idx
              ON task_timer_sessions(task_id, stopped_at, completed_at);
            CREATE TABLE IF NOT EXISTS saved_task_views (
              id TEXT PRIMARY KEY NOT NULL,
              name TEXT NOT NULL,
              slug TEXT NOT NULL UNIQUE,
              description TEXT,
              filter_json TEXT NOT NULL,
              sort_json TEXT NOT NULL,
              is_system INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              archived_at TEXT
            );
            CREATE TABLE IF NOT EXISTS scoring_settings (
              id TEXT PRIMARY KEY NOT NULL,
              priority_weight INTEGER NOT NULL,
              pinned_boost INTEGER NOT NULL,
              overdue_boost INTEGER NOT NULL,
              due_soon_boost INTEGER NOT NULL,
              in_progress_boost INTEGER NOT NULL,
              blocked_penalty INTEGER NOT NULL,
              waiting_penalty INTEGER NOT NULL,
              paused_project_penalty INTEGER NOT NULL,
              due_soon_window_hours INTEGER NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
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
            "#,
        )?;
        self.ensure_sync_event_ownership_columns()?;
        self.seed_system_saved_task_views()?;
        self.ensure_default_scoring_settings()?;
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

    fn seed_system_saved_task_views(&self) -> Result<()> {
        let now = Utc::now();
        let views = [
            (
                "All Tasks",
                "all-tasks",
                serde_json::json!({"include_done": true}),
            ),
            ("Today", "today", serde_json::json!({"due": "today"})),
            ("MVP", "mvp", serde_json::json!({"tags": ["mvp"]})),
            ("Launch", "launch", serde_json::json!({"tags": ["launch"]})),
            ("Bugs", "bugs", serde_json::json!({"tags": ["bug"]})),
            (
                "Blocked",
                "blocked",
                serde_json::json!({"status": ["blocked"]}),
            ),
            ("Due Soon", "due-soon", serde_json::json!({"due": "soon"})),
            (
                "In Progress",
                "in-progress",
                serde_json::json!({"status": ["in_progress"]}),
            ),
            ("Pinned", "pinned", serde_json::json!({"pinned": true})),
        ];
        let connection = self.connection()?;
        for (name, slug, filter_json) in views {
            let id = format!("system-{slug}");
            connection.execute(
                "INSERT INTO saved_task_views (
                    id,name,slug,description,filter_json,sort_json,is_system,created_at,updated_at,archived_at
                 ) VALUES (?1,?2,?3,NULL,?4,?5,1,?6,?6,NULL)
                 ON CONFLICT(slug) DO UPDATE SET
                    name=excluded.name,
                    filter_json=excluded.filter_json,
                    sort_json=excluded.sort_json,
                    is_system=1,
                    archived_at=NULL,
                    updated_at=excluded.updated_at",
                params![
                    id,
                    name,
                    slug,
                    filter_json.to_string(),
                    serde_json::json!({"field": "urgency", "descending": true}).to_string(),
                    timestamp(now),
                ],
            )?;
        }
        Ok(())
    }

    fn ensure_default_scoring_settings(&self) -> Result<()> {
        let exists: bool = self.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM scoring_settings WHERE id='default')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            let connection = self.connection()?;
            save_scoring_settings_with_connection(
                &connection,
                &default_scoring_settings(Utc::now()),
            )?;
        }
        Ok(())
    }

    fn scoring_weights(&self) -> Result<ScoringWeights> {
        Ok(scoring_settings_to_weights(&self.get_scoring_settings()?))
    }

    fn task_context_rows(&self) -> Result<Vec<(TaskContext, String, String, Option<String>)>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT t.id,t.project_id,t.title,t.description,t.status,t.priority,t.due_at,
              t.scheduled_at,t.started_at,t.completed_at,t.estimated_minutes,t.time_limit_minutes,
              t.pinned,t.blocked_reason,t.tags,t.created_at,t.updated_at,
              p.id,p.name,p.type,p.status,p.priority,o.id,o.name,o.color,o.icon
             FROM tasks t JOIN projects p ON p.id=t.project_id
             JOIN organizations o ON o.id=p.organization_id
             WHERE p.archived_at IS NULL AND p.status != 'archived' AND o.archived_at IS NULL",
        )?;
        Ok(statement
            .query_map([], |row| {
                let task = map_task(row)?;
                let project_id: String = row.get(17)?;
                let project_name: String = row.get(18)?;
                let project_type = parse_enum(row.get::<_, String>(19)?)?;
                let project_status = parse_enum(row.get::<_, String>(20)?)?;
                let project_priority = row.get(21)?;
                let organization_id: String = row.get(22)?;
                let organization_name: String = row.get(23)?;
                let organization_color = row.get(24)?;
                let organization_icon = row.get(25)?;
                Ok((
                    TaskContext {
                        task,
                        project_name,
                        project_type,
                        project_status,
                        project_priority,
                        organization_name,
                        organization_color,
                    },
                    project_id,
                    organization_id,
                    organization_icon,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn active_timer_map(&self, now: DateTime<Utc>) -> Result<HashMap<String, ActiveTimerInfo>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,task_id,started_at,paused_at,resumed_at,stopped_at,completed_at,
                    duration_seconds,note,created_at,updated_at
             FROM task_timer_sessions
             WHERE stopped_at IS NULL AND completed_at IS NULL",
        )?;
        let mut map = HashMap::new();
        for session in statement.query_map([], map_timer_session)? {
            let session = session?;
            map.insert(
                session.task_id.clone(),
                ActiveTimerInfo {
                    session_id: session.id.clone(),
                    started_at: session.started_at,
                    paused_at: session.paused_at,
                    resumed_at: session.resumed_at,
                    elapsed_seconds: timer_elapsed_seconds(&session, now),
                    is_running: session.paused_at.is_none(),
                },
            );
        }
        Ok(map)
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

    pub fn apply_remote_sync_event(&self, event: &SyncEvent) -> Result<RemoteApplyResult> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let local_device_id = get_or_create_device_id_with_connection(&transaction)?;
        let status = if event.device_id == local_device_id {
            RemoteApplyStatus::SkippedLocalEcho
        } else if transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM applied_remote_events WHERE event_id=?1)",
            [&event.event_id],
            |row| row.get::<_, bool>(0),
        )? {
            RemoteApplyStatus::AlreadyApplied
        } else {
            apply_remote_domain_change(&transaction, event)?;
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
            RemoteApplyStatus::Applied
        };
        transaction.commit()?;
        Ok(RemoteApplyResult {
            event_id: event.event_id.clone(),
            entity_type: event.entity_type,
            entity_id: event.entity_id.clone(),
            operation: event.operation,
            status,
        })
    }

    pub fn apply_remote_sync_events(&self, events: &[SyncEvent]) -> Result<RemoteApplyBatchResult> {
        let mut result = RemoteApplyBatchResult {
            applied_count: 0,
            already_applied_count: 0,
            skipped_local_echo_count: 0,
            results: Vec::with_capacity(events.len()),
        };
        let mut ordered_events = events.iter().collect::<Vec<_>>();
        ordered_events.sort_by_key(|event| remote_apply_dependency_rank(event.entity_type));
        for event in ordered_events {
            let applied = self.apply_remote_sync_event(event)?;
            match applied.status {
                RemoteApplyStatus::Applied => result.applied_count += 1,
                RemoteApplyStatus::AlreadyApplied => result.already_applied_count += 1,
                RemoteApplyStatus::SkippedLocalEcho => result.skipped_local_echo_count += 1,
            }
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
             ORDER BY p.priority ASC,p.name",
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
             ORDER BY t.priority ASC,t.created_at",
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

    pub fn start_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut task = get_task_with_connection(&transaction, task_id)?;
        if matches!(task.status, TaskStatus::Done | TaskStatus::Canceled) {
            return Err(CoreError::Validation(
                "cannot start a timer on a done or canceled task".into(),
            ));
        }
        if get_active_timer_session_with_connection(&transaction, task_id)?.is_some() {
            return Err(CoreError::Validation(
                "task already has an active timer session".into(),
            ));
        }
        let now = Utc::now();
        if task.status != TaskStatus::InProgress {
            task.status = TaskStatus::InProgress;
            task.updated_at = now;
            if task.started_at.is_none() {
                task.started_at = Some(now);
            }
            save_task_with_connection(&transaction, &task)?;
            append_sync_event_with_connection(
                &transaction,
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Transitioned,
                serde_json::json!({ "entity": &task }),
            )?;
        }
        let session = TaskTimerSession {
            id: Uuid::new_v4().to_string(),
            task_id: task_id.to_owned(),
            started_at: now,
            paused_at: None,
            resumed_at: None,
            stopped_at: None,
            completed_at: None,
            duration_seconds: Some(0),
            note: None,
            created_at: now,
            updated_at: now,
        };
        insert_timer_session_with_connection(&transaction, &session)?;
        transaction.commit()?;
        Ok(session)
    }

    pub fn pause_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut session = get_active_timer_session_with_connection(&transaction, task_id)?
            .ok_or(CoreError::NotFound("active timer session"))?;
        if session.paused_at.is_some() {
            return Ok(session);
        }
        let now = Utc::now();
        session.duration_seconds = Some(timer_elapsed_seconds(&session, now));
        session.paused_at = Some(now);
        session.updated_at = now;
        save_timer_session_with_connection(&transaction, &session)?;
        transaction.commit()?;
        Ok(session)
    }

    pub fn resume_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut session = get_active_timer_session_with_connection(&transaction, task_id)?
            .ok_or(CoreError::NotFound("active timer session"))?;
        if session.paused_at.is_none() {
            return Ok(session);
        }
        let now = Utc::now();
        session.paused_at = None;
        session.resumed_at = Some(now);
        session.updated_at = now;
        save_timer_session_with_connection(&transaction, &session)?;
        transaction.commit()?;
        Ok(session)
    }

    pub fn stop_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut session = get_active_timer_session_with_connection(&transaction, task_id)?
            .ok_or(CoreError::NotFound("active timer session"))?;
        let now = Utc::now();
        session.duration_seconds = Some(timer_elapsed_seconds(&session, now));
        session.stopped_at = Some(now);
        session.updated_at = now;
        save_timer_session_with_connection(&transaction, &session)?;
        transaction.commit()?;
        Ok(session)
    }

    pub fn complete_task_with_timer(&self, task_id: &str) -> Result<Task> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        if let Some(mut session) = get_active_timer_session_with_connection(&transaction, task_id)?
        {
            let now = Utc::now();
            session.duration_seconds = Some(timer_elapsed_seconds(&session, now));
            session.completed_at = Some(now);
            session.updated_at = now;
            save_timer_session_with_connection(&transaction, &session)?;
        }
        let mut task = get_task_with_connection(&transaction, task_id)?;
        let now = Utc::now();
        task.status = TaskStatus::Done;
        task.completed_at = Some(now);
        task.updated_at = now;
        task.blocked_reason = None;
        save_task_with_connection(&transaction, &task)?;
        append_sync_event_with_connection(
            &transaction,
            SyncEntityType::Task,
            &task.id,
            SyncOperation::Transitioned,
            serde_json::json!({ "entity": &task }),
        )?;
        transaction.commit()?;
        Ok(task)
    }

    pub fn list_task_timer_sessions(&self, task_id: &str) -> Result<Vec<TaskTimerSession>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,task_id,started_at,paused_at,resumed_at,stopped_at,completed_at,
                    duration_seconds,note,created_at,updated_at
             FROM task_timer_sessions WHERE task_id=?1 ORDER BY started_at DESC",
        )?;
        Ok(statement
            .query_map([task_id], map_timer_session)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_active_timer_session(&self, task_id: &str) -> Result<Option<TaskTimerSession>> {
        let connection = self.connection()?;
        get_active_timer_session_with_connection(&connection, task_id)
    }

    pub fn list_saved_task_views(&self) -> Result<Vec<SavedTaskView>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,name,slug,description,filter_json,sort_json,is_system,created_at,updated_at,archived_at
             FROM saved_task_views WHERE archived_at IS NULL ORDER BY is_system DESC, name COLLATE NOCASE",
        )?;
        Ok(statement
            .query_map([], map_saved_task_view)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_saved_task_view(&self, id: &str) -> Result<SavedTaskView> {
        let connection = self.connection()?;
        get_saved_task_view_with_connection(&connection, id)
    }

    pub fn create_saved_task_view(&self, input: NewSavedTaskView) -> Result<SavedTaskView> {
        require_name(&input.name)?;
        let now = Utc::now();
        let slug = normalized_saved_view_slug(input.slug, &input.name)?;
        let view = SavedTaskView {
            id: Uuid::new_v4().to_string(),
            slug,
            name: input.name,
            description: input.description,
            filter_json: input.filter_json,
            sort_json: input.sort_json,
            is_system: false,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        let connection = self.connection()?;
        insert_saved_task_view_with_connection(&connection, &view)?;
        Ok(view)
    }

    pub fn update_saved_task_view(
        &self,
        id: &str,
        patch: SavedTaskViewPatch,
    ) -> Result<SavedTaskView> {
        let connection = self.connection()?;
        let mut view = get_saved_task_view_with_connection(&connection, id)?;
        if view.is_system {
            return Err(CoreError::Validation(
                "system saved task views cannot be edited".into(),
            ));
        }
        if let Some(name) = patch.name {
            require_name(&name)?;
            view.name = name;
        }
        if let Some(slug) = patch.slug {
            view.slug = normalized_saved_view_slug(Some(slug), &view.name)?;
        }
        if let Some(description) = patch.description {
            view.description = description;
        }
        if let Some(filter_json) = patch.filter_json {
            view.filter_json = filter_json;
        }
        if let Some(sort_json) = patch.sort_json {
            view.sort_json = sort_json;
        }
        view.updated_at = Utc::now();
        changed(
            connection.execute(
                "UPDATE saved_task_views SET name=?2,slug=?3,description=?4,filter_json=?5,
                 sort_json=?6,updated_at=?7 WHERE id=?1",
                params![
                    view.id,
                    view.name,
                    view.slug,
                    view.description,
                    view.filter_json.to_string(),
                    view.sort_json.to_string(),
                    timestamp(view.updated_at),
                ],
            )?,
            "saved task view",
        )?;
        Ok(view)
    }

    pub fn archive_saved_task_view(&self, id: &str) -> Result<()> {
        let view = self.get_saved_task_view(id)?;
        if view.is_system {
            return Err(CoreError::Validation(
                "system saved task views cannot be archived".into(),
            ));
        }
        let now = Utc::now();
        changed(
            self.connection()?.execute(
                "UPDATE saved_task_views SET archived_at=?2,updated_at=?2 WHERE id=?1",
                params![id, timestamp(now)],
            )?,
            "saved task view",
        )
    }

    pub fn query_tasks(
        &self,
        filter: TaskQueryFilter,
        sort: Option<TaskSort>,
    ) -> Result<Vec<TaskWithContext>> {
        let now = Utc::now();
        let weights = self.scoring_weights()?;
        let active_timers = self.active_timer_map(now)?;
        let mut rows = self.task_context_rows()?;
        rows.retain(|row| task_matches_filter(row, &filter, now));
        let mut rows = rows
            .into_iter()
            .map(
                |(task_context, project_id, organization_id, organization_icon)| {
                    let urgency_score = score_task(&task_context, now, weights);
                    let active_timer = active_timers.get(&task_context.task.id).cloned();
                    TaskWithContext {
                        project_id,
                        project_name: task_context.project_name.clone(),
                        project_type: task_context.project_type,
                        organization_id,
                        organization_name: task_context.organization_name.clone(),
                        organization_color: task_context.organization_color.clone(),
                        organization_icon,
                        task: task_context.task,
                        urgency_score,
                        active_timer,
                    }
                },
            )
            .collect::<Vec<_>>();
        sort_task_rows(&mut rows, sort.unwrap_or_default());
        Ok(rows)
    }

    pub fn get_scoring_settings(&self) -> Result<ScoringSettings> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id,priority_weight,pinned_boost,overdue_boost,due_soon_boost,
                        in_progress_boost,blocked_penalty,waiting_penalty,paused_project_penalty,
                        due_soon_window_hours,created_at,updated_at
                 FROM scoring_settings WHERE id='default'",
                [],
                map_scoring_settings,
            )
            .optional()?
            .ok_or(CoreError::NotFound("scoring settings"))
    }

    pub fn update_scoring_settings(&self, patch: ScoringSettingsPatch) -> Result<ScoringSettings> {
        let mut settings = self.get_scoring_settings()?;
        if let Some(value) = patch.priority_weight {
            settings.priority_weight = value;
        }
        if let Some(value) = patch.pinned_boost {
            settings.pinned_boost = value;
        }
        if let Some(value) = patch.overdue_boost {
            settings.overdue_boost = value;
        }
        if let Some(value) = patch.due_soon_boost {
            settings.due_soon_boost = value;
        }
        if let Some(value) = patch.in_progress_boost {
            settings.in_progress_boost = value;
        }
        if let Some(value) = patch.blocked_penalty {
            settings.blocked_penalty = value;
        }
        if let Some(value) = patch.waiting_penalty {
            settings.waiting_penalty = value;
        }
        if let Some(value) = patch.paused_project_penalty {
            settings.paused_project_penalty = value;
        }
        if let Some(value) = patch.due_soon_window_hours {
            settings.due_soon_window_hours = value.max(1);
        }
        settings.updated_at = Utc::now();
        let connection = self.connection()?;
        save_scoring_settings_with_connection(&connection, &settings)?;
        Ok(settings)
    }

    pub fn reset_scoring_settings(&self) -> Result<ScoringSettings> {
        let settings = default_scoring_settings(Utc::now());
        let connection = self.connection()?;
        save_scoring_settings_with_connection(&connection, &settings)?;
        Ok(settings)
    }

    pub fn export_tasks_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.query_tasks(TaskQueryFilter::default(), None)?)
            .map_err(|error| CoreError::Validation(format!("could not export tasks: {error}")))
    }

    pub fn export_tasks_csv(&self) -> Result<String> {
        let rows = self.query_tasks(TaskQueryFilter::default(), None)?;
        let mut csv = String::from(
            "id,title,status,priority,project,organization,due_at,scheduled_at,completed_at,tags,urgency_score\n",
        );
        for row in rows {
            let tags = row.task.tags.join(";");
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_cell(&row.task.id),
                csv_cell(&row.task.title),
                csv_cell(&row.task.status.to_string()),
                row.task.priority,
                csv_cell(&row.project_name),
                csv_cell(&row.organization_name),
                csv_cell(&row.task.due_at.map(timestamp).unwrap_or_default()),
                csv_cell(&row.task.scheduled_at.map(timestamp).unwrap_or_default()),
                csv_cell(&row.task.completed_at.map(timestamp).unwrap_or_default()),
                csv_cell(&tags),
                row.urgency_score,
            ));
        }
        Ok(csv)
    }

    pub fn export_all_json(&self) -> Result<String> {
        let value = serde_json::json!({
            "organizations": self.list_organizations()?,
            "projects": self.list_projects()?,
            "tasks": self.query_tasks(TaskQueryFilter {
                include_done: Some(true),
                include_canceled: Some(true),
                ..Default::default()
            }, None)?,
            "saved_task_views": self.list_saved_task_views()?,
            "scoring_settings": self.get_scoring_settings()?,
        });
        serde_json::to_string_pretty(&value)
            .map_err(|error| CoreError::Validation(format!("could not export data: {error}")))
    }

    pub fn backup_sqlite_database(&self, target_path: impl AsRef<Path>) -> Result<()> {
        let target = target_path.as_ref();
        if let Some(parent) = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        self.connection()?
            .execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])?;
        Ok(())
    }

    pub fn board_state(&self) -> Result<BoardState> {
        let contexts = self
            .task_context_rows()?
            .into_iter()
            .map(|(context, _, _, _)| context)
            .collect::<Vec<_>>();
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
                    // P1 = highest priority.
                    priority: 1,
                    deadline: None,
                    repo_url: Some("https://github.com/LaneBucher/openmgmt".into()),
                    notes: None,
                },
                MutationOrigin::Seed,
            )
        })?;

        let now = Utc::now();
        // Priorities use P1 = highest .. P5 = lowest, so the most urgent seed
        // work (in-progress MVP, overdue decision) is P1/P2.
        let seeds = [
            (
                "Review the MVP on the TV board",
                TaskStatus::InProgress,
                1,
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
                2,
                Some(now - Duration::hours(3)),
                None,
                false,
                None,
            ),
            (
                "Confirm external dependency",
                TaskStatus::Blocked,
                2,
                Some(now + Duration::hours(8)),
                None,
                false,
                Some("Waiting for confirmation"),
            ),
            (
                "Plan the afternoon review",
                TaskStatus::Scheduled,
                4,
                Some(now + Duration::hours(30)),
                Some(now + Duration::hours(4)),
                false,
                None,
            ),
            (
                "Capture launch notes",
                TaskStatus::Inbox,
                4,
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

fn get_active_timer_session_with_connection(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<TaskTimerSession>> {
    connection
        .query_row(
            "SELECT id,task_id,started_at,paused_at,resumed_at,stopped_at,completed_at,
                    duration_seconds,note,created_at,updated_at
             FROM task_timer_sessions
             WHERE task_id=?1 AND stopped_at IS NULL AND completed_at IS NULL
             ORDER BY started_at DESC LIMIT 1",
            [task_id],
            map_timer_session,
        )
        .optional()
        .map_err(Into::into)
}

fn insert_timer_session_with_connection(
    connection: &Connection,
    session: &TaskTimerSession,
) -> Result<()> {
    connection.execute(
        "INSERT INTO task_timer_sessions (
            id,task_id,started_at,paused_at,resumed_at,stopped_at,completed_at,
            duration_seconds,note,created_at,updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            session.id,
            session.task_id,
            timestamp(session.started_at),
            session.paused_at.map(timestamp),
            session.resumed_at.map(timestamp),
            session.stopped_at.map(timestamp),
            session.completed_at.map(timestamp),
            session.duration_seconds,
            session.note,
            timestamp(session.created_at),
            timestamp(session.updated_at),
        ],
    )?;
    Ok(())
}

fn save_timer_session_with_connection(
    connection: &Connection,
    session: &TaskTimerSession,
) -> Result<()> {
    changed(
        connection.execute(
            "UPDATE task_timer_sessions SET paused_at=?3,resumed_at=?4,stopped_at=?5,
             completed_at=?6,duration_seconds=?7,note=?8,updated_at=?9 WHERE id=?1 AND task_id=?2",
            params![
                session.id,
                session.task_id,
                session.paused_at.map(timestamp),
                session.resumed_at.map(timestamp),
                session.stopped_at.map(timestamp),
                session.completed_at.map(timestamp),
                session.duration_seconds,
                session.note,
                timestamp(session.updated_at),
            ],
        )?,
        "timer session",
    )
}

fn timer_elapsed_seconds(session: &TaskTimerSession, now: DateTime<Utc>) -> i64 {
    let stored = session.duration_seconds.unwrap_or(0).max(0);
    if session.paused_at.is_some() || session.stopped_at.is_some() || session.completed_at.is_some()
    {
        return stored;
    }
    let run_started = session.resumed_at.unwrap_or(session.started_at);
    stored + (now - run_started).num_seconds().max(0)
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

fn map_timer_session(row: &Row<'_>) -> rusqlite::Result<TaskTimerSession> {
    Ok(TaskTimerSession {
        id: row.get(0)?,
        task_id: row.get(1)?,
        started_at: parse_time(row.get(2)?)?,
        paused_at: parse_optional_time(row.get(3)?)?,
        resumed_at: parse_optional_time(row.get(4)?)?,
        stopped_at: parse_optional_time(row.get(5)?)?,
        completed_at: parse_optional_time(row.get(6)?)?,
        duration_seconds: row.get(7)?,
        note: row.get(8)?,
        created_at: parse_time(row.get(9)?)?,
        updated_at: parse_time(row.get(10)?)?,
    })
}

fn map_saved_task_view(row: &Row<'_>) -> rusqlite::Result<SavedTaskView> {
    let filter_json = row.get::<_, String>(4)?;
    let sort_json = row.get::<_, String>(5)?;
    Ok(SavedTaskView {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        description: row.get(3)?,
        filter_json: serde_json::from_str(&filter_json).unwrap_or(serde_json::Value::Null),
        sort_json: serde_json::from_str(&sort_json).unwrap_or(serde_json::Value::Null),
        is_system: row.get(6)?,
        created_at: parse_time(row.get(7)?)?,
        updated_at: parse_time(row.get(8)?)?,
        archived_at: parse_optional_time(row.get(9)?)?,
    })
}

fn insert_saved_task_view_with_connection(
    connection: &Connection,
    view: &SavedTaskView,
) -> Result<()> {
    connection.execute(
        "INSERT INTO saved_task_views (
            id,name,slug,description,filter_json,sort_json,is_system,created_at,updated_at,archived_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            view.id,
            view.name,
            view.slug,
            view.description,
            view.filter_json.to_string(),
            view.sort_json.to_string(),
            view.is_system,
            timestamp(view.created_at),
            timestamp(view.updated_at),
            view.archived_at.map(timestamp),
        ],
    )?;
    Ok(())
}

fn get_saved_task_view_with_connection(connection: &Connection, id: &str) -> Result<SavedTaskView> {
    connection
        .query_row(
            "SELECT id,name,slug,description,filter_json,sort_json,is_system,created_at,updated_at,archived_at
             FROM saved_task_views WHERE id=?1",
            [id],
            map_saved_task_view,
        )
        .optional()?
        .ok_or(CoreError::NotFound("saved task view"))
}

fn normalized_saved_view_slug(input: Option<String>, name: &str) -> Result<String> {
    match input {
        Some(value) => {
            let slug = slugify(&value);
            if slug.is_empty() {
                Err(CoreError::Validation(
                    "saved task view slug must contain letters or numbers".into(),
                ))
            } else {
                Ok(slug)
            }
        }
        None => {
            let slug = slugify(name);
            if slug.is_empty() {
                Err(CoreError::Validation(
                    "saved task view name must produce a valid slug".into(),
                ))
            } else {
                Ok(slug)
            }
        }
    }
}

fn map_scoring_settings(row: &Row<'_>) -> rusqlite::Result<ScoringSettings> {
    Ok(ScoringSettings {
        id: row.get(0)?,
        priority_weight: row.get(1)?,
        pinned_boost: row.get(2)?,
        overdue_boost: row.get(3)?,
        due_soon_boost: row.get(4)?,
        in_progress_boost: row.get(5)?,
        blocked_penalty: row.get(6)?,
        waiting_penalty: row.get(7)?,
        paused_project_penalty: row.get(8)?,
        due_soon_window_hours: row.get(9)?,
        created_at: parse_time(row.get(10)?)?,
        updated_at: parse_time(row.get(11)?)?,
    })
}

fn save_scoring_settings_with_connection(
    connection: &Connection,
    settings: &ScoringSettings,
) -> Result<()> {
    connection.execute(
        "INSERT INTO scoring_settings (
            id,priority_weight,pinned_boost,overdue_boost,due_soon_boost,in_progress_boost,
            blocked_penalty,waiting_penalty,paused_project_penalty,due_soon_window_hours,
            created_at,updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(id) DO UPDATE SET
            priority_weight=excluded.priority_weight,
            pinned_boost=excluded.pinned_boost,
            overdue_boost=excluded.overdue_boost,
            due_soon_boost=excluded.due_soon_boost,
            in_progress_boost=excluded.in_progress_boost,
            blocked_penalty=excluded.blocked_penalty,
            waiting_penalty=excluded.waiting_penalty,
            paused_project_penalty=excluded.paused_project_penalty,
            due_soon_window_hours=excluded.due_soon_window_hours,
            updated_at=excluded.updated_at",
        params![
            settings.id,
            settings.priority_weight,
            settings.pinned_boost,
            settings.overdue_boost,
            settings.due_soon_boost,
            settings.in_progress_boost,
            settings.blocked_penalty,
            settings.waiting_penalty,
            settings.paused_project_penalty,
            settings.due_soon_window_hours,
            timestamp(settings.created_at),
            timestamp(settings.updated_at),
        ],
    )?;
    Ok(())
}

fn default_scoring_settings(now: DateTime<Utc>) -> ScoringSettings {
    let weights = ScoringWeights::default();
    ScoringSettings {
        id: "default".into(),
        priority_weight: weights.priority_step,
        pinned_boost: weights.pinned,
        overdue_boost: weights.overdue_base,
        due_soon_boost: weights.due_today,
        in_progress_boost: weights.in_progress,
        blocked_penalty: weights.blocked,
        waiting_penalty: weights.waiting,
        paused_project_penalty: weights.paused_project,
        due_soon_window_hours: 24,
        created_at: now,
        updated_at: now,
    }
}

fn scoring_settings_to_weights(settings: &ScoringSettings) -> ScoringWeights {
    ScoringWeights {
        priority_step: settings.priority_weight,
        project_priority_step: ScoringWeights::default().project_priority_step,
        pinned: settings.pinned_boost,
        overdue_base: settings.overdue_boost,
        overdue_per_day: ScoringWeights::default().overdue_per_day,
        due_within_hour: settings.due_soon_boost + 20,
        due_today: settings.due_soon_boost,
        due_tomorrow: settings.due_soon_boost / 2,
        due_soon_window_hours: settings.due_soon_window_hours as i64,
        in_progress: settings.in_progress_boost,
        ready: ScoringWeights::default().ready,
        blocked: settings.blocked_penalty,
        waiting: settings.waiting_penalty,
        paused_project: settings.paused_project_penalty,
    }
}

fn task_matches_filter(
    row: &(TaskContext, String, String, Option<String>),
    filter: &TaskQueryFilter,
    now: DateTime<Utc>,
) -> bool {
    let (context, project_id, organization_id, _) = row;
    let task = &context.task;
    if !filter.include_canceled.unwrap_or(false) && task.status == TaskStatus::Canceled {
        return false;
    }
    if !filter.include_done.unwrap_or(false)
        && matches!(task.status, TaskStatus::Done | TaskStatus::Canceled)
    {
        return false;
    }
    if let Some(value) = &filter.organization_id
        && organization_id != value
    {
        return false;
    }
    if let Some(value) = &filter.project_id
        && project_id != value
    {
        return false;
    }
    if let Some(statuses) = &filter.status
        && !statuses.contains(&task.status)
    {
        return false;
    }
    if let Some(priorities) = &filter.priority
        && !priorities.contains(&task.priority)
    {
        return false;
    }
    if let Some(from) = filter.due_from
        && !task.due_at.is_some_and(|value| value >= from)
    {
        return false;
    }
    if let Some(to) = filter.due_to
        && !task.due_at.is_some_and(|value| value <= to)
    {
        return false;
    }
    if let Some(from) = filter.scheduled_from
        && !task.scheduled_at.is_some_and(|value| value >= from)
    {
        return false;
    }
    if let Some(to) = filter.scheduled_to
        && !task.scheduled_at.is_some_and(|value| value <= to)
    {
        return false;
    }
    if let Some(pinned) = filter.pinned
        && task.pinned != pinned
    {
        return false;
    }
    if let Some(tags) = &filter.tags
        && !tags
            .iter()
            .all(|tag| task.tags.iter().any(|item| item.eq_ignore_ascii_case(tag)))
    {
        return false;
    }
    if let Some(text) = &filter.text {
        let text = text.to_lowercase();
        let haystack = format!(
            "{} {} {} {} {}",
            task.title,
            task.description.clone().unwrap_or_default(),
            context.project_name,
            context.organization_name,
            task.tags.join(" ")
        )
        .to_lowercase();
        if !haystack.contains(&text) {
            return false;
        }
    }
    if task.status == TaskStatus::Done {
        return task
            .completed_at
            .is_some_and(|completed| completed.date_naive() == now.date_naive())
            || filter.include_done.unwrap_or(false);
    }
    true
}

fn sort_task_rows(rows: &mut [TaskWithContext], sort: TaskSort) {
    rows.sort_by(|a, b| {
        let ordering = match sort.field {
            TaskSortField::Urgency => a.urgency_score.cmp(&b.urgency_score),
            TaskSortField::Priority => b.task.priority.cmp(&a.task.priority),
            TaskSortField::DueAt => cmp_option_time(a.task.due_at, b.task.due_at),
            TaskSortField::Status => a.task.status.to_string().cmp(&b.task.status.to_string()),
            TaskSortField::Project => a.project_name.cmp(&b.project_name),
            TaskSortField::Organization => a.organization_name.cmp(&b.organization_name),
            TaskSortField::CreatedAt => a.task.created_at.cmp(&b.task.created_at),
            TaskSortField::UpdatedAt => a.task.updated_at.cmp(&b.task.updated_at),
            TaskSortField::Tag => a.task.tags.first().cmp(&b.task.tags.first()),
        };
        let ordering = if sort.descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering
            .then_with(|| a.task.priority.cmp(&b.task.priority))
            .then_with(|| a.task.created_at.cmp(&b.task.created_at))
    });
}

fn cmp_option_time(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
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
        RemoteApplyStatus, SyncConnectionState, SyncEntityType, SyncOperation, SyncSettingsPatch,
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

    #[test]
    fn archived_saved_task_views_are_hidden_from_list() {
        let db = Database::in_memory().unwrap();
        let view = db
            .create_saved_task_view(NewSavedTaskView {
                name: "Archive Me".into(),
                slug: None,
                description: None,
                filter_json: serde_json::json!({}),
                sort_json: serde_json::json!({}),
            })
            .unwrap();

        db.archive_saved_task_view(&view.id).unwrap();

        assert!(
            !db.list_saved_task_views()
                .unwrap()
                .iter()
                .any(|item| item.id == view.id)
        );
        assert!(
            db.get_saved_task_view(&view.id)
                .unwrap()
                .archived_at
                .is_some()
        );
    }

    #[test]
    fn saved_task_view_slugs_are_normalized_and_validated() {
        let db = Database::in_memory().unwrap();
        let custom = db
            .create_saved_task_view(NewSavedTaskView {
                name: "Custom".into(),
                slug: Some("  Mixed Case Slug  ".into()),
                description: None,
                filter_json: serde_json::json!({}),
                sort_json: serde_json::json!({}),
            })
            .unwrap();
        assert_eq!(custom.slug, "mixed-case-slug");

        let missing = db
            .create_saved_task_view(NewSavedTaskView {
                name: "Missing Slug View".into(),
                slug: None,
                description: None,
                filter_json: serde_json::json!({}),
                sort_json: serde_json::json!({}),
            })
            .unwrap();
        assert_eq!(missing.slug, "missing-slug-view");

        for slug in ["   ", "!!!"] {
            assert!(matches!(
                db.create_saved_task_view(NewSavedTaskView {
                    name: format!("Bad {slug}"),
                    slug: Some(slug.into()),
                    description: None,
                    filter_json: serde_json::json!({}),
                    sort_json: serde_json::json!({}),
                }),
                Err(CoreError::Validation(_))
            ));
        }
    }

    #[test]
    fn update_saved_task_view_completes_and_normalizes_slug() {
        let db = Database::in_memory().unwrap();
        let view = db
            .create_saved_task_view(NewSavedTaskView {
                name: "Editable".into(),
                slug: None,
                description: None,
                filter_json: serde_json::json!({}),
                sort_json: serde_json::json!({}),
            })
            .unwrap();

        let updated = db
            .update_saved_task_view(
                &view.id,
                SavedTaskViewPatch {
                    name: Some("Updated".into()),
                    slug: Some("Updated Slug".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(updated.name, "Updated");
        assert_eq!(updated.slug, "updated-slug");
    }

    #[test]
    fn scoring_due_soon_window_affects_query_urgency() {
        let db = seeded_database();
        let task = db
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Canceled))
            .unwrap();
        db.update_task(
            &task.id,
            TaskPatch {
                due_at: Some(Some(Utc::now() + Duration::hours(10))),
                ..Default::default()
            },
        )
        .unwrap();

        let wide_score = db
            .query_tasks(
                TaskQueryFilter {
                    include_done: Some(true),
                    ..Default::default()
                },
                None,
            )
            .unwrap()
            .into_iter()
            .find(|row| row.task.id == task.id)
            .unwrap()
            .urgency_score;
        db.update_scoring_settings(ScoringSettingsPatch {
            due_soon_window_hours: Some(4),
            ..Default::default()
        })
        .unwrap();
        let narrow_score = db
            .query_tasks(
                TaskQueryFilter {
                    include_done: Some(true),
                    ..Default::default()
                },
                None,
            )
            .unwrap()
            .into_iter()
            .find(|row| row.task.id == task.id)
            .unwrap()
            .urgency_score;

        assert!(wide_score > narrow_score);
    }
}
