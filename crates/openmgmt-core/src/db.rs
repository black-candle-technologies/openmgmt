#[cfg(test)]
use crate::models::{ProjectStatus, ProjectType};
use crate::{
    board::build_board,
    models::{
        ActiveTimerInfo, BoardState, CalendarBlock, CalendarBlockSource, CalendarBlockStatus,
        GptActionLog, NewGptActionLog, NewOrganization, NewProject, NewSavedTaskView, NewTask,
        Organization, OrganizationPatch, Project, ProjectPatch, RecurrenceRule, SavedTaskView,
        SavedTaskViewPatch, ScheduleConflict, ScheduleTaskInput, ScheduledBlockCompletion,
        ScoringSettings, ScoringSettingsPatch, Task, TaskContext, TaskPatch, TaskQueryFilter,
        TaskSort, TaskSortField, TaskStatus, TaskTimerSession, TaskWithContext,
        TimeBlockSuggestion,
    },
    scheduling::{generate_schedule_ics, next_recurrence_at},
    scoring::{ScoringWeights, score_task},
    sync::{
        DEFAULT_DEVICE_NAME, LOCAL_DEVICE_ID_KEY, RemoteApplyBatchResult, RemoteApplyResult,
        RemoteApplyStatus, SYNC_ACCOUNT_ID_KEY, SYNC_DEVICE_NAME_KEY, SYNC_DEVICE_TOKEN_KEY,
        SYNC_ENABLED_KEY, SYNC_LAST_ATTEMPTED_AT_KEY, SYNC_LAST_ERROR_KEY,
        SYNC_LAST_SUCCESSFUL_AT_KEY, SYNC_SERVER_URL_KEY, SYNC_USER_ID_KEY, SyncConnectionState,
        SyncEntityType, SyncEvent, SyncOperation, SyncSettings, SyncSettingsPatch, SyncStatus,
    },
};
#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, TimeZone, Utc};
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
    #[cfg(test)]
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
              due_at TEXT, scheduled_at TEXT, scheduled_start_at TEXT, scheduled_end_at TEXT,
              deadline_at TEXT, reminder_at TEXT, recurrence_rule TEXT, recurrence_anchor_at TEXT,
              recurrence_timezone TEXT, calendar_block_id TEXT, started_at TEXT, completed_at TEXT,
              estimated_minutes INTEGER, time_limit_minutes INTEGER, pinned INTEGER NOT NULL DEFAULT 0,
              blocked_reason TEXT, tags TEXT NOT NULL DEFAULT '[]',
              created_at TEXT NOT NULL, updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS tasks_project_idx ON tasks(project_id);
            CREATE INDEX IF NOT EXISTS tasks_status_idx ON tasks(status);
            CREATE INDEX IF NOT EXISTS tasks_due_idx ON tasks(due_at);
            CREATE TABLE IF NOT EXISTS calendar_blocks (
              id TEXT PRIMARY KEY NOT NULL,
              task_id TEXT REFERENCES tasks(id),
              project_id TEXT REFERENCES projects(id),
              organization_id TEXT REFERENCES organizations(id),
              title TEXT NOT NULL,
              description TEXT,
              start_at TEXT NOT NULL,
              end_at TEXT NOT NULL,
              timezone TEXT,
              source TEXT NOT NULL,
              external_id TEXT,
              status TEXT NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS calendar_blocks_start_idx ON calendar_blocks(start_at);
            CREATE INDEX IF NOT EXISTS calendar_blocks_end_idx ON calendar_blocks(end_at);
            CREATE INDEX IF NOT EXISTS calendar_blocks_task_idx ON calendar_blocks(task_id);
            CREATE INDEX IF NOT EXISTS calendar_blocks_project_idx ON calendar_blocks(project_id);
            CREATE INDEX IF NOT EXISTS calendar_blocks_organization_idx
              ON calendar_blocks(organization_id);
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
            CREATE TABLE IF NOT EXISTS gpt_action_log (
              id TEXT PRIMARY KEY NOT NULL,
              created_at TEXT NOT NULL,
              action TEXT NOT NULL,
              resource_type TEXT NOT NULL,
              resource_id TEXT,
              method TEXT NOT NULL,
              path TEXT NOT NULL,
              request_summary TEXT NOT NULL,
              success INTEGER NOT NULL,
              error_message TEXT
            );
            CREATE INDEX IF NOT EXISTS gpt_action_log_created_idx
              ON gpt_action_log(created_at);
            CREATE INDEX IF NOT EXISTS gpt_action_log_resource_idx
              ON gpt_action_log(resource_type, resource_id);
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
        self.ensure_task_scheduling_columns()?;
        self.ensure_sync_event_ownership_columns()?;
        self.seed_system_saved_task_views()?;
        self.ensure_default_scoring_settings()?;
        Ok(())
    }

    fn ensure_task_scheduling_columns(&self) -> Result<()> {
        let connection = self.connection()?;
        let columns = connection
            .prepare("PRAGMA table_info(tasks)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (column, definition) in [
            ("scheduled_start_at", "TEXT"),
            ("scheduled_end_at", "TEXT"),
            ("deadline_at", "TEXT"),
            ("reminder_at", "TEXT"),
            ("recurrence_rule", "TEXT"),
            ("recurrence_anchor_at", "TEXT"),
            ("recurrence_timezone", "TEXT"),
            ("calendar_block_id", "TEXT"),
        ] {
            if !columns.iter().any(|existing| existing == column) {
                connection.execute(
                    &format!("ALTER TABLE tasks ADD COLUMN {column} {definition}"),
                    [],
                )?;
            }
        }
        if columns.iter().any(|existing| existing == "scheduled_at") {
            connection.execute(
                "UPDATE tasks
                 SET scheduled_start_at = scheduled_at
                 WHERE scheduled_start_at IS NULL AND scheduled_at IS NOT NULL",
                [],
            )?;
        }
        connection.execute_batch(
            "CREATE INDEX IF NOT EXISTS tasks_scheduled_start_idx ON tasks(scheduled_start_at);
             CREATE INDEX IF NOT EXISTS tasks_scheduled_end_idx ON tasks(scheduled_end_at);
             CREATE INDEX IF NOT EXISTS tasks_reminder_idx ON tasks(reminder_at);",
        )?;
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
              t.scheduled_at,t.scheduled_start_at,t.scheduled_end_at,t.deadline_at,t.reminder_at,
              t.recurrence_rule,t.recurrence_anchor_at,t.recurrence_timezone,t.calendar_block_id,
              t.started_at,t.completed_at,t.estimated_minutes,t.time_limit_minutes,
              t.pinned,t.blocked_reason,t.tags,t.created_at,t.updated_at,
              p.id,p.name,p.type,p.status,p.priority,o.id,o.name,o.color,o.icon
             FROM tasks t JOIN projects p ON p.id=t.project_id
             JOIN organizations o ON o.id=p.organization_id
             WHERE p.archived_at IS NULL AND p.status != 'archived' AND o.archived_at IS NULL",
        )?;
        Ok(statement
            .query_map([], |row| {
                let task = map_task(row)?;
                let project_id: String = row.get(25)?;
                let project_name: String = row.get(26)?;
                let project_type = parse_enum(row.get::<_, String>(27)?)?;
                let project_status = parse_enum(row.get::<_, String>(28)?)?;
                let project_priority = row.get(29)?;
                let organization_id: String = row.get(30)?;
                let organization_name: String = row.get(31)?;
                let organization_color = row.get(32)?;
                let organization_icon = row.get(33)?;
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
                    t.scheduled_at,t.scheduled_start_at,t.scheduled_end_at,t.deadline_at,
                    t.reminder_at,t.recurrence_rule,t.recurrence_anchor_at,t.recurrence_timezone,
                    t.calendar_block_id,t.started_at,t.completed_at,t.estimated_minutes,
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
            scheduled_start_at: input.scheduled_at,
            scheduled_end_at: input.scheduled_at.and_then(|start| {
                input
                    .estimated_minutes
                    .map(|minutes| start + ChronoDuration::minutes(minutes.into()))
            }),
            deadline_at: None,
            reminder_at: None,
            recurrence_rule: None,
            recurrence_anchor_at: None,
            recurrence_timezone: None,
            calendar_block_id: None,
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
        let started_at = task.started_at.map(timestamp);
        let completed_at = task.completed_at.map(timestamp);
        let tags = serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".into());
        let created_at = timestamp(task.created_at);
        let updated_at = timestamp(task.updated_at);
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        get_project_with_connection(&transaction, &task.project_id)?;
        transaction.execute(
            "INSERT INTO tasks (
                id,project_id,title,description,status,priority,due_at,scheduled_at,
                scheduled_start_at,scheduled_end_at,deadline_at,reminder_at,recurrence_rule,
                recurrence_anchor_at,recurrence_timezone,calendar_block_id,started_at,completed_at,
                estimated_minutes,time_limit_minutes,pinned,blocked_reason,tags,created_at,updated_at
             ) VALUES (
                ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
                ?19,?20,?21,?22,?23,?24,?25
             )",
            params![
                task.id,
                task.project_id,
                task.title,
                task.description,
                status,
                task.priority,
                due_at,
                task.scheduled_at.map(timestamp),
                task.scheduled_start_at.map(timestamp),
                task.scheduled_end_at.map(timestamp),
                task.deadline_at.map(timestamp),
                task.reminder_at.map(timestamp),
                task.recurrence_rule.map(|value| value.to_string()),
                task.recurrence_anchor_at.map(timestamp),
                task.recurrence_timezone,
                task.calendar_block_id,
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
            task.scheduled_start_at = value;
            task.scheduled_end_at = value.and_then(|start| {
                task.estimated_minutes
                    .map(|minutes| start + ChronoDuration::minutes(minutes.into()))
            });
        }
        if let Some(value) = patch.estimated_minutes {
            task.estimated_minutes = value;
            if let Some(start) = task.scheduled_start_at {
                task.scheduled_end_at =
                    value.map(|minutes| start + ChronoDuration::minutes(minutes.into()));
            }
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

    pub fn get_schedule_today(&self) -> Result<Vec<TaskWithContext>> {
        let now = Utc::now();
        let start = Utc.from_utc_datetime(
            &now.date_naive()
                .and_hms_opt(0, 0, 0)
                .expect("valid midnight"),
        );
        self.scheduled_tasks_between(start, start + ChronoDuration::days(1))
    }

    pub fn get_schedule_week(&self) -> Result<Vec<TaskWithContext>> {
        let now = Utc::now();
        let today = now.date_naive();
        let monday = today - ChronoDuration::days(today.weekday().num_days_from_monday().into());
        let start = Utc.from_utc_datetime(&monday.and_hms_opt(0, 0, 0).expect("valid midnight"));
        self.scheduled_tasks_between(start, start + ChronoDuration::days(7))
    }

    /// Scheduled tasks whose start falls within an explicit `[start, end)` window.
    ///
    /// Backs the Schedule page's day navigation: the UI computes the selected
    /// local day's (or week's) UTC window and asks for exactly that range, so the
    /// timeline is no longer pinned to the real "today".
    pub fn get_schedule_for_day(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<TaskWithContext>> {
        self.scheduled_tasks_between(start, end)
    }

    pub fn get_unscheduled_tasks(&self) -> Result<Vec<TaskWithContext>> {
        Ok(self
            .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?
            .into_iter()
            .filter(|item| item.task.scheduled_start_at.is_none())
            .collect())
    }

    pub fn get_overdue_tasks(&self) -> Result<Vec<TaskWithContext>> {
        let now = Utc::now();
        Ok(self
            .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?
            .into_iter()
            .filter(|item| {
                item.task.due_at.is_some_and(|value| value < now)
                    || item
                        .task
                        .scheduled_end_at
                        .or(item.task.scheduled_start_at)
                        .is_some_and(|value| value <= now)
            })
            .collect())
    }

    /// Start any scheduled task whose time block is currently active.
    ///
    /// "Active" means `scheduled_start_at <= now < scheduled_end_at`. A task is
    /// only started when it is in a plannable state (not done, canceled, already
    /// in progress, blocked, or waiting) and has no active timer session, so this
    /// is idempotent: once a task is started it has an active timer and is skipped
    /// on every subsequent call. Reuses [`start_task_timer`] so the task moves to
    /// `InProgress` and a timer session is opened exactly as a manual start would.
    /// Returns the tasks that were auto-started this call (empty when none are due).
    pub fn auto_start_due_scheduled_tasks(&self) -> Result<Vec<Task>> {
        let now = Utc::now();
        let due: Vec<String> = self
            .list_tasks()?
            .into_iter()
            .filter(|task| {
                matches!(
                    (task.scheduled_start_at, task.scheduled_end_at),
                    (Some(start), Some(end)) if start <= now && now < end
                ) && !matches!(
                    task.status,
                    TaskStatus::Done
                        | TaskStatus::Canceled
                        | TaskStatus::InProgress
                        | TaskStatus::Blocked
                        | TaskStatus::Waiting
                )
            })
            .map(|task| task.id)
            .collect();

        let mut started = Vec::new();
        for task_id in due {
            // Skip anything already being timed; `start_task_timer` would also
            // reject it, but checking first keeps this loop free of spurious errors.
            if self.get_active_timer_session(&task_id)?.is_some() {
                continue;
            }
            self.start_task_timer(&task_id)?;
            started.push(self.get_task(&task_id)?);
        }
        Ok(started)
    }

    pub fn schedule_task(&self, task_id: &str, input: ScheduleTaskInput) -> Result<CalendarBlock> {
        self.schedule_task_internal(task_id, input, false)
    }

    pub fn reschedule_task(
        &self,
        task_id: &str,
        input: ScheduleTaskInput,
    ) -> Result<CalendarBlock> {
        self.schedule_task_internal(task_id, input, true)
    }

    fn schedule_task_internal(
        &self,
        task_id: &str,
        input: ScheduleTaskInput,
        preserve_moved_block: bool,
    ) -> Result<CalendarBlock> {
        validate_schedule_range(input.start_at, input.end_at)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut task = get_task_with_connection(&transaction, task_id)?;
        if matches!(task.status, TaskStatus::Done | TaskStatus::Canceled) {
            return Err(CoreError::Validation(
                "done or canceled tasks cannot be scheduled".into(),
            ));
        }
        let project = get_project_with_connection(&transaction, &task.project_id)?;
        let now = Utc::now();
        if let Some(block_id) = task.calendar_block_id.as_deref() {
            if preserve_moved_block {
                transaction.execute(
                    "UPDATE calendar_blocks SET status='moved',updated_at=?2 WHERE id=?1",
                    params![block_id, timestamp(now)],
                )?;
            } else {
                transaction.execute("DELETE FROM calendar_blocks WHERE id=?1", [block_id])?;
            }
        }
        let block = CalendarBlock {
            id: Uuid::new_v4().to_string(),
            task_id: Some(task.id.clone()),
            project_id: Some(project.id),
            organization_id: Some(project.organization_id),
            title: task.title.clone(),
            description: task.description.clone(),
            start_at: input.start_at,
            end_at: input.end_at,
            timezone: clean_optional_string(input.timezone),
            source: CalendarBlockSource::OpenMgmt,
            external_id: None,
            status: CalendarBlockStatus::Planned,
            created_at: now,
            updated_at: now,
        };
        insert_calendar_block_with_connection(&transaction, &block)?;
        task.scheduled_at = Some(input.start_at);
        task.scheduled_start_at = Some(input.start_at);
        task.scheduled_end_at = Some(input.end_at);
        task.deadline_at = input.deadline_at;
        task.reminder_at = input.reminder_at;
        task.recurrence_rule = input
            .recurrence_rule
            .filter(|rule| *rule != RecurrenceRule::None);
        task.recurrence_anchor_at = input.recurrence_anchor_at.or(Some(input.start_at));
        task.recurrence_timezone = clean_optional_string(input.recurrence_timezone);
        task.calendar_block_id = Some(block.id.clone());
        if matches!(
            task.status,
            TaskStatus::Inbox | TaskStatus::Backlog | TaskStatus::Ready
        ) {
            task.status = TaskStatus::Scheduled;
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
        Ok(block)
    }

    pub fn clear_task_schedule(&self, task_id: &str) -> Result<Task> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut task = get_task_with_connection(&transaction, task_id)?;
        if let Some(block_id) = task.calendar_block_id.as_deref() {
            transaction.execute(
                "UPDATE calendar_blocks SET status='canceled',updated_at=?2 WHERE id=?1",
                params![block_id, timestamp(Utc::now())],
            )?;
        }
        clear_task_schedule_fields(&mut task);
        if task.status == TaskStatus::Scheduled {
            task.status = TaskStatus::Ready;
        }
        task.updated_at = Utc::now();
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

    pub fn list_schedule_conflicts(&self) -> Result<Vec<ScheduleConflict>> {
        let blocks = self
            .list_calendar_blocks()?
            .into_iter()
            .filter(|block| block.status == CalendarBlockStatus::Planned)
            .collect::<Vec<_>>();
        let mut conflicts = Vec::new();
        for (index, first) in blocks.iter().enumerate() {
            for second in blocks.iter().skip(index + 1) {
                if first.start_at < second.end_at && second.start_at < first.end_at {
                    conflicts.push(ScheduleConflict {
                        first: first.clone(),
                        second: second.clone(),
                    });
                }
            }
        }
        Ok(conflicts)
    }

    pub fn suggest_next_time_block(
        &self,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        duration_minutes: i64,
    ) -> Result<Option<TimeBlockSuggestion>> {
        if duration_minutes <= 0 {
            return Err(CoreError::Validation(
                "duration_minutes must be greater than zero".into(),
            ));
        }
        validate_schedule_range(window_start, window_end)?;
        let duration = ChronoDuration::minutes(duration_minutes);
        let mut cursor = window_start;
        let mut blocks = self
            .list_calendar_blocks()?
            .into_iter()
            .filter(|block| {
                block.status == CalendarBlockStatus::Planned
                    && block.end_at > window_start
                    && block.start_at < window_end
            })
            .collect::<Vec<_>>();
        blocks.sort_by_key(|block| block.start_at);
        for block in blocks {
            if cursor + duration <= block.start_at {
                return Ok(Some(TimeBlockSuggestion {
                    start_at: cursor,
                    end_at: cursor + duration,
                    duration_minutes,
                }));
            }
            if block.end_at > cursor {
                cursor = block.end_at;
            }
        }
        Ok(
            (cursor + duration <= window_end).then_some(TimeBlockSuggestion {
                start_at: cursor,
                end_at: cursor + duration,
                duration_minutes,
            }),
        )
    }

    pub fn suggest_tasks_for_time_window(
        &self,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<Vec<TaskWithContext>> {
        validate_schedule_range(window_start, window_end)?;
        let available_minutes = (window_end - window_start).num_minutes();
        Ok(self
            .get_unscheduled_tasks()?
            .into_iter()
            .filter(|item| {
                item.task.estimated_minutes.unwrap_or(30).clamp(1, i32::MAX) as i64
                    <= available_minutes
            })
            .collect())
    }

    pub fn complete_scheduled_block(&self, block_id: &str) -> Result<ScheduledBlockCompletion> {
        let mut block = self.get_calendar_block(block_id)?;
        block.status = CalendarBlockStatus::Completed;
        block.updated_at = Utc::now();
        self.save_calendar_block(&block)?;
        let mut task = None;
        let mut next_occurrence_task = None;
        if let Some(task_id) = block.task_id.as_deref() {
            let completed = self.complete_task_with_timer(task_id)?;
            if let Some(rule) = completed.recurrence_rule
                && let Some(next_start) = next_recurrence_at(rule, block.start_at)
            {
                let duration = block.end_at - block.start_at;
                let next = self.create_task(NewTask {
                    project_id: completed.project_id.clone(),
                    title: completed.title.clone(),
                    description: completed.description.clone(),
                    status: TaskStatus::Scheduled,
                    priority: completed.priority,
                    due_at: completed
                        .due_at
                        .map(|due| due + (next_start - block.start_at)),
                    scheduled_at: Some(next_start),
                    estimated_minutes: completed.estimated_minutes,
                    time_limit_minutes: completed.time_limit_minutes,
                    pinned: completed.pinned,
                    tags: completed.tags.clone(),
                })?;
                self.schedule_task(
                    &next.id,
                    ScheduleTaskInput {
                        start_at: next_start,
                        end_at: next_start + duration,
                        timezone: block.timezone.clone(),
                        reminder_at: completed
                            .reminder_at
                            .map(|reminder| reminder + (next_start - block.start_at)),
                        deadline_at: completed
                            .deadline_at
                            .map(|deadline| deadline + (next_start - block.start_at)),
                        recurrence_rule: Some(rule),
                        recurrence_anchor_at: Some(next_start),
                        recurrence_timezone: completed.recurrence_timezone.clone(),
                    },
                )?;
                next_occurrence_task = Some(self.get_task(&next.id)?);
            }
            task = Some(completed);
        }
        Ok(ScheduledBlockCompletion {
            block,
            task,
            next_occurrence_task,
        })
    }

    pub fn skip_scheduled_block(&self, block_id: &str) -> Result<CalendarBlock> {
        let mut block = self.get_calendar_block(block_id)?;
        block.status = CalendarBlockStatus::Skipped;
        block.updated_at = Utc::now();
        self.save_calendar_block(&block)?;
        if let Some(task_id) = block.task_id.as_deref() {
            let mut connection = self.connection()?;
            let transaction = connection.transaction()?;
            let mut task = get_task_with_connection(&transaction, task_id)?;
            clear_task_schedule_fields(&mut task);
            if task.status == TaskStatus::Scheduled {
                task.status = TaskStatus::Ready;
            }
            task.updated_at = Utc::now();
            save_task_with_connection(&transaction, &task)?;
            append_sync_event_with_connection(
                &transaction,
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Updated,
                serde_json::json!({ "entity": &task }),
            )?;
            transaction.commit()?;
        }
        Ok(block)
    }

    pub fn generate_schedule_ics(&self) -> Result<String> {
        Ok(generate_schedule_ics(&self.list_calendar_blocks()?))
    }

    pub fn list_calendar_blocks(&self) -> Result<Vec<CalendarBlock>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,task_id,project_id,organization_id,title,description,start_at,end_at,
                    timezone,source,external_id,status,created_at,updated_at
             FROM calendar_blocks ORDER BY start_at",
        )?;
        Ok(statement
            .query_map([], map_calendar_block)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn get_calendar_block(&self, block_id: &str) -> Result<CalendarBlock> {
        self.connection()?
            .query_row(
                "SELECT id,task_id,project_id,organization_id,title,description,start_at,end_at,
                        timezone,source,external_id,status,created_at,updated_at
                 FROM calendar_blocks WHERE id=?1",
                [block_id],
                map_calendar_block,
            )
            .optional()?
            .ok_or(CoreError::NotFound("calendar block"))
    }

    fn save_calendar_block(&self, block: &CalendarBlock) -> Result<()> {
        changed(
            self.connection()?.execute(
                "UPDATE calendar_blocks SET task_id=?2,project_id=?3,organization_id=?4,title=?5,
                 description=?6,start_at=?7,end_at=?8,timezone=?9,source=?10,external_id=?11,
                 status=?12,created_at=?13,updated_at=?14 WHERE id=?1",
                params![
                    block.id,
                    block.task_id,
                    block.project_id,
                    block.organization_id,
                    block.title,
                    block.description,
                    timestamp(block.start_at),
                    timestamp(block.end_at),
                    block.timezone,
                    block.source.to_string(),
                    block.external_id,
                    block.status.to_string(),
                    timestamp(block.created_at),
                    timestamp(block.updated_at),
                ],
            )?,
            "calendar block",
        )
    }

    fn scheduled_tasks_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<TaskWithContext>> {
        Ok(self
            .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?
            .into_iter()
            .filter(|item| {
                item.task
                    .scheduled_start_at
                    .is_some_and(|scheduled| scheduled >= start && scheduled < end)
            })
            .collect())
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

    pub fn record_gpt_action(&self, input: NewGptActionLog) -> Result<GptActionLog> {
        let now = Utc::now();
        let log = GptActionLog {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            action: input.action,
            resource_type: input.resource_type,
            resource_id: input.resource_id,
            method: input.method,
            path: input.path,
            request_summary: input.request_summary,
            success: input.success,
            error_message: input.error_message,
        };
        self.connection()?.execute(
            "INSERT INTO gpt_action_log (
                id,created_at,action,resource_type,resource_id,method,path,
                request_summary,success,error_message
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                log.id,
                timestamp(log.created_at),
                log.action,
                log.resource_type,
                log.resource_id,
                log.method,
                log.path,
                log.request_summary,
                log.success as i32,
                log.error_message
            ],
        )?;
        Ok(log)
    }

    pub fn list_gpt_action_logs(&self) -> Result<Vec<GptActionLog>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,created_at,action,resource_type,resource_id,method,path,
                    request_summary,success,error_message
             FROM gpt_action_log ORDER BY created_at DESC",
        )?;
        Ok(statement
            .query_map([], map_gpt_action_log)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn board_state(&self) -> Result<BoardState> {
        self.board_state_at(Utc::now())
    }

    fn board_state_at(&self, now: DateTime<Utc>) -> Result<BoardState> {
        let contexts = self
            .task_context_rows()?
            .into_iter()
            .map(|(context, _, _, _)| context)
            .collect::<Vec<_>>();
        Ok(build_board(contexts, now))
    }

    #[cfg(test)]
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
 scheduled_at,scheduled_start_at,scheduled_end_at,deadline_at,reminder_at,recurrence_rule,
 recurrence_anchor_at,recurrence_timezone,calendar_block_id,started_at,completed_at,
 estimated_minutes,time_limit_minutes,pinned,blocked_reason,tags,created_at,updated_at FROM tasks";

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
            id,project_id,title,description,status,priority,due_at,scheduled_at,
            scheduled_start_at,scheduled_end_at,deadline_at,reminder_at,recurrence_rule,
            recurrence_anchor_at,recurrence_timezone,calendar_block_id,started_at,completed_at,
            estimated_minutes,time_limit_minutes,pinned,blocked_reason,tags,created_at,updated_at
         ) VALUES (
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
            ?19,?20,?21,?22,?23,?24,?25
         )
         ON CONFLICT(id) DO UPDATE SET
            project_id=excluded.project_id,title=excluded.title,description=excluded.description,
            status=excluded.status,priority=excluded.priority,due_at=excluded.due_at,
            scheduled_at=excluded.scheduled_at,scheduled_start_at=excluded.scheduled_start_at,
            scheduled_end_at=excluded.scheduled_end_at,deadline_at=excluded.deadline_at,
            reminder_at=excluded.reminder_at,recurrence_rule=excluded.recurrence_rule,
            recurrence_anchor_at=excluded.recurrence_anchor_at,
            recurrence_timezone=excluded.recurrence_timezone,
            calendar_block_id=excluded.calendar_block_id,started_at=excluded.started_at,
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
            task.scheduled_start_at.map(timestamp),
            task.scheduled_end_at.map(timestamp),
            task.deadline_at.map(timestamp),
            task.reminder_at.map(timestamp),
            task.recurrence_rule.map(|value| value.to_string()),
            task.recurrence_anchor_at.map(timestamp),
            task.recurrence_timezone,
            task.calendar_block_id,
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
    let tags = serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".into());
    changed(
        connection.execute(
            "UPDATE tasks SET project_id=?2,title=?3,description=?4,status=?5,priority=?6,due_at=?7,
             scheduled_at=?8,scheduled_start_at=?9,scheduled_end_at=?10,deadline_at=?11,
             reminder_at=?12,recurrence_rule=?13,recurrence_anchor_at=?14,
             recurrence_timezone=?15,calendar_block_id=?16,started_at=?17,completed_at=?18,
             estimated_minutes=?19,time_limit_minutes=?20,pinned=?21,blocked_reason=?22,
             tags=?23,created_at=?24,updated_at=?25 WHERE id=?1",
            params![
                task.id,
                task.project_id,
                task.title,
                task.description,
                status,
                task.priority,
                task.due_at.map(timestamp),
                task.scheduled_at.map(timestamp),
                task.scheduled_start_at.map(timestamp),
                task.scheduled_end_at.map(timestamp),
                task.deadline_at.map(timestamp),
                task.reminder_at.map(timestamp),
                task.recurrence_rule.map(|value| value.to_string()),
                task.recurrence_anchor_at.map(timestamp),
                task.recurrence_timezone,
                task.calendar_block_id,
                task.started_at.map(timestamp),
                task.completed_at.map(timestamp),
                task.estimated_minutes,
                task.time_limit_minutes,
                task.pinned,
                task.blocked_reason,
                tags,
                timestamp(task.created_at),
                timestamp(task.updated_at)
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
    let tags: String = row.get(22)?;
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: parse_enum(row.get::<_, String>(4)?)?,
        priority: row.get(5)?,
        due_at: parse_optional_time(row.get(6)?)?,
        scheduled_at: parse_optional_time(row.get(7)?)?,
        scheduled_start_at: parse_optional_time(row.get(8)?)?,
        scheduled_end_at: parse_optional_time(row.get(9)?)?,
        deadline_at: parse_optional_time(row.get(10)?)?,
        reminder_at: parse_optional_time(row.get(11)?)?,
        recurrence_rule: row
            .get::<_, Option<String>>(12)?
            .map(parse_enum)
            .transpose()?,
        recurrence_anchor_at: parse_optional_time(row.get(13)?)?,
        recurrence_timezone: row.get(14)?,
        calendar_block_id: row.get(15)?,
        started_at: parse_optional_time(row.get(16)?)?,
        completed_at: parse_optional_time(row.get(17)?)?,
        estimated_minutes: row.get(18)?,
        time_limit_minutes: row.get(19)?,
        pinned: row.get(20)?,
        blocked_reason: row.get(21)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        created_at: parse_time(row.get(23)?)?,
        updated_at: parse_time(row.get(24)?)?,
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

fn map_calendar_block(row: &Row<'_>) -> rusqlite::Result<CalendarBlock> {
    Ok(CalendarBlock {
        id: row.get(0)?,
        task_id: row.get(1)?,
        project_id: row.get(2)?,
        organization_id: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        start_at: parse_time(row.get(6)?)?,
        end_at: parse_time(row.get(7)?)?,
        timezone: row.get(8)?,
        source: parse_enum(row.get::<_, String>(9)?)?,
        external_id: row.get(10)?,
        status: parse_enum(row.get::<_, String>(11)?)?,
        created_at: parse_time(row.get(12)?)?,
        updated_at: parse_time(row.get(13)?)?,
    })
}

fn insert_calendar_block_with_connection(
    connection: &Connection,
    block: &CalendarBlock,
) -> Result<()> {
    connection.execute(
        "INSERT INTO calendar_blocks (
            id,task_id,project_id,organization_id,title,description,start_at,end_at,timezone,
            source,external_id,status,created_at,updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            block.id,
            block.task_id,
            block.project_id,
            block.organization_id,
            block.title,
            block.description,
            timestamp(block.start_at),
            timestamp(block.end_at),
            block.timezone,
            block.source.to_string(),
            block.external_id,
            block.status.to_string(),
            timestamp(block.created_at),
            timestamp(block.updated_at),
        ],
    )?;
    Ok(())
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

fn map_gpt_action_log(row: &Row<'_>) -> rusqlite::Result<GptActionLog> {
    Ok(GptActionLog {
        id: row.get(0)?,
        created_at: parse_time(row.get(1)?)?,
        action: row.get(2)?,
        resource_type: row.get(3)?,
        resource_id: row.get(4)?,
        method: row.get(5)?,
        path: row.get(6)?,
        request_summary: row.get(7)?,
        success: row.get(8)?,
        error_message: row.get(9)?,
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
        && !task
            .scheduled_start_at
            .or(task.scheduled_at)
            .is_some_and(|value| value >= from)
    {
        return false;
    }
    if let Some(to) = filter.scheduled_to
        && !task
            .scheduled_start_at
            .or(task.scheduled_at)
            .is_some_and(|value| value <= to)
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

fn validate_schedule_range(start_at: DateTime<Utc>, end_at: DateTime<Utc>) -> Result<()> {
    if end_at <= start_at {
        Err(CoreError::Validation(
            "scheduled end must be after scheduled start".into(),
        ))
    } else {
        Ok(())
    }
}

fn clear_task_schedule_fields(task: &mut Task) {
    task.scheduled_at = None;
    task.scheduled_start_at = None;
    task.scheduled_end_at = None;
    task.deadline_at = None;
    task.reminder_at = None;
    task.recurrence_rule = None;
    task.recurrence_anchor_at = None;
    task.recurrence_timezone = None;
    task.calendar_block_id = None;
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

    fn scheduling_task(db: &Database, title: &str, priority: i32, minutes: i32) -> Task {
        let organization = db
            .list_organizations()
            .unwrap()
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                db.create_organization(NewOrganization {
                    name: "Scheduling Org".into(),
                    slug: None,
                    description: None,
                    color: None,
                    icon: None,
                })
                .unwrap()
            });
        let project = db
            .list_projects()
            .unwrap()
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                db.create_project(NewProject {
                    organization_id: organization.id,
                    name: "Scheduling Project".into(),
                    slug: None,
                    description: None,
                    project_type: ProjectType::Operations,
                    status: ProjectStatus::Active,
                    priority: 3,
                    deadline: None,
                    repo_url: None,
                    notes: None,
                })
                .unwrap()
            });
        db.create_task(NewTask {
            project_id: project.id,
            title: title.into(),
            description: None,
            status: TaskStatus::Ready,
            priority,
            due_at: None,
            scheduled_at: None,
            estimated_minutes: Some(minutes),
            time_limit_minutes: None,
            pinned: false,
            tags: Vec::new(),
        })
        .unwrap()
    }

    fn schedule_input(start_at: DateTime<Utc>, end_at: DateTime<Utc>) -> ScheduleTaskInput {
        ScheduleTaskInput {
            start_at,
            end_at,
            timezone: Some("America/Chicago".into()),
            reminder_at: None,
            deadline_at: None,
            recurrence_rule: None,
            recurrence_anchor_at: None,
            recurrence_timezone: None,
        }
    }

    #[test]
    fn fresh_database_has_no_user_domain_records() {
        let db = Database::in_memory().unwrap();

        assert!(db.list_organizations().unwrap().is_empty());
        assert!(db.list_projects().unwrap().is_empty());
        assert!(db.list_tasks().unwrap().is_empty());
    }

    #[test]
    fn fresh_database_keeps_required_settings_and_empty_board() {
        let db = Database::in_memory().unwrap();

        assert_eq!(db.get_scoring_settings().unwrap().id, "default");
        assert!(!db.list_saved_task_views().unwrap().is_empty());
        let board = db.board_state().unwrap();
        assert!(board.now.is_empty());
        assert!(board.next_up.is_empty());
        assert!(board.due_soon.is_empty());
        assert!(board.waiting_blocked.is_empty());
        assert!(board.later_today.is_empty());
        assert!(board.overdue.is_empty());
        assert!(board.done_today.is_empty());
    }

    #[test]
    fn fresh_database_exports_empty_user_data() {
        let db = Database::in_memory().unwrap();

        let tasks_json = db.export_tasks_json().unwrap();
        let tasks: Vec<TaskWithContext> = serde_json::from_str(&tasks_json).unwrap();
        assert!(tasks.is_empty());

        let all_json = db.export_all_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&all_json).unwrap();
        assert_eq!(value["organizations"].as_array().unwrap().len(), 0);
        assert_eq!(value["projects"].as_array().unwrap().len(), 0);
        assert_eq!(value["tasks"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn migration_adds_scheduling_columns_and_calendar_blocks() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE tasks (
                    id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, title TEXT NOT NULL,
                    description TEXT, status TEXT NOT NULL, priority INTEGER NOT NULL,
                    due_at TEXT, scheduled_at TEXT, started_at TEXT, completed_at TEXT,
                    estimated_minutes INTEGER, time_limit_minutes INTEGER,
                    pinned INTEGER NOT NULL DEFAULT 0, blocked_reason TEXT,
                    tags TEXT NOT NULL DEFAULT '[]', created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .unwrap();
        let db = Database {
            connection: Arc::new(Mutex::new(connection)),
        };
        db.migrate().unwrap();
        let connection = db.connection().unwrap();
        let columns = connection
            .prepare("PRAGMA table_info(tasks)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for column in [
            "scheduled_start_at",
            "scheduled_end_at",
            "deadline_at",
            "reminder_at",
            "recurrence_rule",
            "recurrence_anchor_at",
            "recurrence_timezone",
            "calendar_block_id",
        ] {
            assert!(columns.iter().any(|existing| existing == column));
        }
        let calendar_blocks_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table'
                 AND name='calendar_blocks')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(calendar_blocks_exists);
    }

    #[test]
    fn migration_backfills_legacy_scheduled_at_without_overwriting_start() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE tasks (
                    id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, title TEXT NOT NULL,
                    description TEXT, status TEXT NOT NULL, priority INTEGER NOT NULL,
                    due_at TEXT, scheduled_at TEXT, scheduled_start_at TEXT,
                    started_at TEXT, completed_at TEXT,
                    estimated_minutes INTEGER, time_limit_minutes INTEGER,
                    pinned INTEGER NOT NULL DEFAULT 0, blocked_reason TEXT,
                    tags TEXT NOT NULL DEFAULT '[]', created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );
                 INSERT INTO tasks (
                    id, project_id, title, description, status, priority, due_at,
                    scheduled_at, scheduled_start_at, started_at, completed_at,
                    estimated_minutes, time_limit_minutes, pinned, blocked_reason,
                    tags, created_at, updated_at
                 ) VALUES
                 (
                    'legacy', 'project', 'Legacy', NULL, 'ready', 3, NULL,
                    '2026-06-19T09:00:00Z', NULL, NULL, NULL,
                    NULL, NULL, 0, NULL, '[]',
                    '2026-06-01T00:00:00Z', '2026-06-01T00:00:00Z'
                 ),
                 (
                    'explicit', 'project', 'Explicit', NULL, 'ready', 3, NULL,
                    '2026-06-19T09:00:00Z', '2026-06-19T10:00:00Z', NULL, NULL,
                    NULL, NULL, 0, NULL, '[]',
                    '2026-06-01T00:00:00Z', '2026-06-01T00:00:00Z'
                 );",
            )
            .unwrap();
        let db = Database {
            connection: Arc::new(Mutex::new(connection)),
        };
        db.migrate().unwrap();
        let connection = db.connection().unwrap();
        let legacy_start: String = connection
            .query_row(
                "SELECT scheduled_start_at FROM tasks WHERE id='legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let explicit_start: String = connection
            .query_row(
                "SELECT scheduled_start_at FROM tasks WHERE id='explicit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_start, "2026-06-19T09:00:00Z");
        assert_eq!(explicit_start, "2026-06-19T10:00:00Z");
    }

    #[test]
    fn schedule_reschedule_and_clear_task() {
        let db = Database::in_memory().unwrap();
        let task = scheduling_task(&db, "Schedule lifecycle", 2, 45);
        let start = Utc::now() + ChronoDuration::hours(1);
        let deadline = start + ChronoDuration::days(1);
        let mut first_input = schedule_input(start, start + ChronoDuration::minutes(45));
        first_input.deadline_at = Some(deadline);
        let first = db.schedule_task(&task.id, first_input).unwrap();
        let scheduled = db.get_task(&task.id).unwrap();
        assert_eq!(scheduled.scheduled_start_at, Some(start));
        assert_eq!(scheduled.deadline_at, Some(deadline));
        assert_eq!(
            scheduled.calendar_block_id.as_deref(),
            Some(first.id.as_str())
        );

        let moved_start = start + ChronoDuration::hours(2);
        let second = db
            .reschedule_task(
                &task.id,
                schedule_input(moved_start, moved_start + ChronoDuration::minutes(45)),
            )
            .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            db.list_calendar_blocks()
                .unwrap()
                .into_iter()
                .find(|block| block.id == first.id)
                .unwrap()
                .status,
            CalendarBlockStatus::Moved
        );

        let cleared = db.clear_task_schedule(&task.id).unwrap();
        assert!(cleared.scheduled_start_at.is_none());
        assert!(cleared.deadline_at.is_none());
        assert!(cleared.calendar_block_id.is_none());
        assert_eq!(cleared.status, TaskStatus::Ready);
    }

    #[test]
    fn today_week_unscheduled_and_overdue_queries_work() {
        let db = Database::in_memory().unwrap();
        let today = scheduling_task(&db, "Today", 2, 30);
        let overdue = scheduling_task(&db, "Overdue", 2, 30);
        let unscheduled = scheduling_task(&db, "Unscheduled", 2, 30);
        // Anchor to noon of the current UTC day. The queries under test bucket by
        // the real UTC day, so using the raw `Utc::now()` made this flaky: in the
        // last ~50 minutes before 00:00 UTC the `today` block (now+20..now+50min)
        // spilled into the next day and dropped out of `get_schedule_today`.
        let now = DateTime::<Utc>::from_naive_utc_and_offset(
            Utc::now()
                .date_naive()
                .and_hms_opt(12, 0, 0)
                .expect("noon is a valid time"),
            Utc,
        );
        db.schedule_task(
            &today.id,
            schedule_input(
                now + ChronoDuration::minutes(20),
                now + ChronoDuration::minutes(50),
            ),
        )
        .unwrap();
        let overdue_now = Utc::now();
        db.schedule_task(
            &overdue.id,
            schedule_input(
                overdue_now - ChronoDuration::hours(2),
                overdue_now - ChronoDuration::hours(1),
            ),
        )
        .unwrap();

        assert!(
            db.get_schedule_today()
                .unwrap()
                .iter()
                .any(|item| item.task.id == today.id)
        );
        assert!(
            db.get_schedule_week()
                .unwrap()
                .iter()
                .any(|item| item.task.id == today.id)
        );
        assert!(
            db.get_unscheduled_tasks()
                .unwrap()
                .iter()
                .any(|item| item.task.id == unscheduled.id)
        );
        assert!(
            db.get_overdue_tasks()
                .unwrap()
                .iter()
                .any(|item| item.task.id == overdue.id)
        );
    }

    #[test]
    fn conflicts_and_next_available_block_are_reported() {
        let db = Database::in_memory().unwrap();
        let first = scheduling_task(&db, "First", 2, 60);
        let second = scheduling_task(&db, "Second", 3, 60);
        let start = Utc::now() + ChronoDuration::hours(1);
        db.schedule_task(
            &first.id,
            schedule_input(start, start + ChronoDuration::hours(1)),
        )
        .unwrap();
        db.schedule_task(
            &second.id,
            schedule_input(
                start + ChronoDuration::minutes(30),
                start + ChronoDuration::minutes(90),
            ),
        )
        .unwrap();
        assert_eq!(db.list_schedule_conflicts().unwrap().len(), 1);

        let suggestion = db
            .suggest_next_time_block(start, start + ChronoDuration::hours(3), 30)
            .unwrap()
            .unwrap();
        assert_eq!(suggestion.start_at, start + ChronoDuration::minutes(90));
    }

    #[test]
    fn task_fit_suggestions_preserve_p1_ordering() {
        let db = Database::in_memory().unwrap();
        let p5 = scheduling_task(&db, "P5", 5, 30);
        let p1 = scheduling_task(&db, "P1", 1, 30);
        scheduling_task(&db, "Too long", 2, 180);
        let start = Utc::now();
        let suggestions = db
            .suggest_tasks_for_time_window(start, start + ChronoDuration::minutes(60))
            .unwrap();
        assert_eq!(suggestions[0].task.id, p1.id);
        assert!(suggestions.iter().any(|item| item.task.id == p5.id));
        assert!(!suggestions.iter().any(|item| item.task.title == "Too long"));
    }

    #[test]
    fn recurring_completion_creates_next_occurrence() {
        let db = Database::in_memory().unwrap();
        let task = scheduling_task(&db, "Daily recurring", 2, 30);
        let start = Utc::now() + ChronoDuration::minutes(10);
        let mut input = schedule_input(start, start + ChronoDuration::minutes(30));
        input.recurrence_rule = Some(RecurrenceRule::Daily);
        let block = db.schedule_task(&task.id, input).unwrap();
        let completion = db.complete_scheduled_block(&block.id).unwrap();
        assert_eq!(completion.task.unwrap().status, TaskStatus::Done);
        let next = completion.next_occurrence_task.unwrap();
        assert_eq!(next.recurrence_rule, Some(RecurrenceRule::Daily));
        assert_eq!(
            next.scheduled_start_at,
            Some(start + ChronoDuration::days(1))
        );
    }

    #[test]
    fn board_respects_current_later_and_overdue_scheduled_tasks() {
        let db = Database::in_memory().unwrap();
        let current = scheduling_task(&db, "Current", 2, 30);
        let later = scheduling_task(&db, "Later", 2, 30);
        let overdue = scheduling_task(&db, "Elapsed", 2, 30);
        let now = Utc
            .with_ymd_and_hms(2026, 6, 19, 12, 0, 0)
            .single()
            .unwrap();
        db.schedule_task(
            &current.id,
            schedule_input(
                now - ChronoDuration::minutes(5),
                now + ChronoDuration::minutes(25),
            ),
        )
        .unwrap();
        db.schedule_task(
            &later.id,
            schedule_input(
                now + ChronoDuration::hours(1),
                now + ChronoDuration::minutes(90),
            ),
        )
        .unwrap();
        db.schedule_task(
            &overdue.id,
            schedule_input(
                now - ChronoDuration::hours(2),
                now - ChronoDuration::hours(1),
            ),
        )
        .unwrap();
        let board = db.board_state_at(now).unwrap();
        assert!(
            board
                .now
                .iter()
                .any(|item| item.context.task.id == current.id)
        );
        assert!(
            board
                .later_today
                .iter()
                .any(|item| item.context.task.id == later.id)
        );
        assert!(
            board
                .overdue
                .iter()
                .any(|item| item.context.task.id == overdue.id)
        );
    }

    #[test]
    fn schedule_for_day_window_filters_by_start() {
        let db = Database::in_memory().unwrap();
        let inside = scheduling_task(&db, "Inside", 2, 30);
        let outside = scheduling_task(&db, "Outside", 2, 30);
        let day_start = Utc::now() + ChronoDuration::days(3);
        let window_start = Utc.from_utc_datetime(
            &day_start
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .expect("midnight"),
        );
        let window_end = window_start + ChronoDuration::days(1);
        db.schedule_task(
            &inside.id,
            schedule_input(
                window_start + ChronoDuration::hours(9),
                window_start + ChronoDuration::hours(10),
            ),
        )
        .unwrap();
        db.schedule_task(
            &outside.id,
            schedule_input(
                window_end + ChronoDuration::hours(9),
                window_end + ChronoDuration::hours(10),
            ),
        )
        .unwrap();

        let rows = db.get_schedule_for_day(window_start, window_end).unwrap();
        assert!(rows.iter().any(|item| item.task.id == inside.id));
        assert!(!rows.iter().any(|item| item.task.id == outside.id));
    }

    #[test]
    fn auto_start_starts_active_block_and_is_idempotent() {
        let db = Database::in_memory().unwrap();
        let active = scheduling_task(&db, "Active now", 2, 30);
        let now = Utc::now();
        db.schedule_task(
            &active.id,
            schedule_input(
                now - ChronoDuration::minutes(5),
                now + ChronoDuration::minutes(25),
            ),
        )
        .unwrap();

        let started = db.auto_start_due_scheduled_tasks().unwrap();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].id, active.id);
        assert_eq!(started[0].status, TaskStatus::InProgress);
        assert!(db.get_active_timer_session(&active.id).unwrap().is_some());

        // Idempotent: the task already has an active timer, so a second tick
        // must not start it again.
        let second = db.auto_start_due_scheduled_tasks().unwrap();
        assert!(second.is_empty());
    }

    #[test]
    fn auto_start_skips_future_elapsed_and_terminal_tasks() {
        let db = Database::in_memory().unwrap();
        let now = Utc::now();

        let future = scheduling_task(&db, "Future", 2, 30);
        db.schedule_task(
            &future.id,
            schedule_input(
                now + ChronoDuration::hours(1),
                now + ChronoDuration::minutes(90),
            ),
        )
        .unwrap();

        let elapsed = scheduling_task(&db, "Elapsed", 2, 30);
        db.schedule_task(
            &elapsed.id,
            schedule_input(
                now - ChronoDuration::hours(2),
                now - ChronoDuration::hours(1),
            ),
        )
        .unwrap();

        let done = scheduling_task(&db, "Done", 2, 30);
        db.schedule_task(
            &done.id,
            schedule_input(
                now - ChronoDuration::minutes(5),
                now + ChronoDuration::minutes(25),
            ),
        )
        .unwrap();
        db.transition_task(&done.id, TaskStatus::Done, None)
            .unwrap();

        let canceled = scheduling_task(&db, "Canceled", 2, 30);
        db.schedule_task(
            &canceled.id,
            schedule_input(
                now - ChronoDuration::minutes(5),
                now + ChronoDuration::minutes(25),
            ),
        )
        .unwrap();
        db.transition_task(&canceled.id, TaskStatus::Canceled, None)
            .unwrap();

        let started = db.auto_start_due_scheduled_tasks().unwrap();
        assert!(started.is_empty());
        assert_eq!(
            db.get_task(&future.id).unwrap().status,
            TaskStatus::Scheduled
        );
        assert_eq!(
            db.get_task(&elapsed.id).unwrap().status,
            TaskStatus::Scheduled
        );
        assert_eq!(db.get_task(&done.id).unwrap().status, TaskStatus::Done);
        assert_eq!(
            db.get_task(&canceled.id).unwrap().status,
            TaskStatus::Canceled
        );
    }

    #[test]
    fn auto_start_with_no_scheduled_tasks_is_noop() {
        let db = Database::in_memory().unwrap();
        scheduling_task(&db, "Unscheduled", 2, 30);
        assert!(db.auto_start_due_scheduled_tasks().unwrap().is_empty());
    }

    #[test]
    fn schedule_ics_contains_basic_calendar_event() {
        let db = Database::in_memory().unwrap();
        let task = scheduling_task(&db, "ICS task", 2, 30);
        let start = Utc::now() + ChronoDuration::hours(1);
        db.schedule_task(
            &task.id,
            schedule_input(start, start + ChronoDuration::minutes(30)),
        )
        .unwrap();
        let ics = db.generate_schedule_ics().unwrap();
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.contains("BEGIN:VEVENT\r\n"));
        assert!(ics.contains("SUMMARY:ICS task\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
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
            scheduled_start_at: None,
            scheduled_end_at: None,
            deadline_at: None,
            reminder_at: None,
            recurrence_rule: None,
            recurrence_anchor_at: None,
            recurrence_timezone: None,
            calendar_block_id: None,
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
