use crate::{
    board::build_board,
    models::{
        BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch, Project,
        ProjectPatch, ProjectStatus, ProjectType, Task, TaskContext, TaskPatch, TaskStatus,
    },
    sync::{LOCAL_DEVICE_ID_KEY, SyncEntityType, SyncEvent, SyncOperation},
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
            "#,
        )?;
        Ok(())
    }

    pub fn get_or_create_device_id(&self) -> Result<String> {
        let connection = self.connection()?;
        if let Some(device_id) = connection
            .query_row(
                "SELECT value FROM sync_state WHERE key=?1",
                [LOCAL_DEVICE_ID_KEY],
                |row| row.get(0),
            )
            .optional()?
        {
            return Ok(device_id);
        }

        let device_id = Uuid::new_v4().to_string();
        connection.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2)",
            params![LOCAL_DEVICE_ID_KEY, device_id],
        )?;
        Ok(device_id)
    }

    pub fn append_sync_event(
        &self,
        entity_type: SyncEntityType,
        entity_id: &str,
        operation: SyncOperation,
        payload_json: serde_json::Value,
    ) -> Result<SyncEvent> {
        let device_id = self.get_or_create_device_id()?;
        let connection = self.connection()?;
        let sequence = connection.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM sync_events WHERE device_id=?1",
            [&device_id],
            |row| row.get(0),
        )?;
        let event = SyncEvent {
            event_id: Uuid::new_v4().to_string(),
            device_id,
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
                event_id,device_id,sequence,entity_type,entity_id,operation,payload_json,
                created_at,synced_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                event.event_id,
                event.device_id,
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

    pub fn list_unsynced_events(&self) -> Result<Vec<SyncEvent>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT event_id,device_id,sequence,entity_type,entity_id,operation,payload_json,
                    created_at,synced_at
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
        self.create_organization_internal(input, true)
    }

    fn create_organization_internal(
        &self,
        input: NewOrganization,
        log_sync: bool,
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
        self.connection()?.execute(
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
        if log_sync {
            self.append_sync_event(
                SyncEntityType::Organization,
                &organization.id,
                SyncOperation::Created,
                serde_json::json!({ "entity": &organization }),
            )?;
        }
        Ok(organization)
    }

    pub fn update_organization(&self, id: &str, patch: OrganizationPatch) -> Result<Organization> {
        let mut current = self.get_organization(id)?;
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
        self.connection()?.execute(
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
        self.append_sync_event(
            SyncEntityType::Organization,
            &current.id,
            SyncOperation::Updated,
            serde_json::json!({ "entity": &current }),
        )?;
        Ok(current)
    }

    pub fn archive_organization(&self, id: &str) -> Result<()> {
        let archived_at = Utc::now();
        changed(
            self.connection()?.execute(
                "UPDATE organizations SET archived_at=?2,updated_at=?2 WHERE id=?1",
                params![id, timestamp(archived_at)],
            )?,
            "organization",
        )?;
        self.append_sync_event(
            SyncEntityType::Organization,
            id,
            SyncOperation::Archived,
            serde_json::json!({ "id": id, "archived_at": archived_at }),
        )?;
        Ok(())
    }

    fn get_organization(&self, id: &str) -> Result<Organization> {
        self.connection()?
            .query_row(
                "SELECT id,name,slug,description,color,icon,created_at,updated_at,archived_at
                 FROM organizations WHERE id=?1",
                [id],
                map_organization,
            )
            .optional()?
            .ok_or(CoreError::NotFound("organization"))
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
        self.connection()?
            .query_row(
                "SELECT id,organization_id,name,slug,description,type,status,priority,deadline,repo_url,
                        notes,created_at,updated_at,archived_at FROM projects WHERE id=?1",
                [id],
                map_project,
            )
            .optional()?
            .ok_or(CoreError::NotFound("project"))
    }

    pub fn create_project(&self, input: NewProject) -> Result<Project> {
        self.create_project_internal(input, true)
    }

    fn create_project_internal(&self, input: NewProject, log_sync: bool) -> Result<Project> {
        require_name(&input.name)?;
        validate_priority(input.priority)?;
        self.get_organization(&input.organization_id)?;
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
        self.connection()?.execute(
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
        if log_sync {
            self.append_sync_event(
                SyncEntityType::Project,
                &project.id,
                SyncOperation::Created,
                serde_json::json!({ "entity": &project }),
            )?;
        }
        Ok(project)
    }

    pub fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project> {
        let mut project = self.get_project(id)?;
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
        self.connection()?.execute(
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
        self.append_sync_event(
            SyncEntityType::Project,
            &project.id,
            SyncOperation::Updated,
            serde_json::json!({ "entity": &project }),
        )?;
        Ok(project)
    }

    pub fn archive_project(&self, id: &str) -> Result<()> {
        let archived_at = Utc::now();
        changed(
            self.connection()?.execute(
                "UPDATE projects SET status='archived',archived_at=?2,updated_at=?2 WHERE id=?1",
                params![id, timestamp(archived_at)],
            )?,
            "project",
        )?;
        self.append_sync_event(
            SyncEntityType::Project,
            id,
            SyncOperation::Archived,
            serde_json::json!({ "id": id, "archived_at": archived_at }),
        )?;
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
        self.connection()?
            .query_row(&format!("{TASK_SELECT} WHERE id=?1"), [id], map_task)
            .optional()?
            .ok_or(CoreError::NotFound("task"))
    }

    pub fn create_task(&self, input: NewTask) -> Result<Task> {
        self.create_task_internal(input, true)
    }

    fn create_task_internal(&self, input: NewTask, log_sync: bool) -> Result<Task> {
        require_name(&input.title)?;
        validate_priority(input.priority)?;
        self.get_project(&input.project_id)?;
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
        self.connection()?.execute(
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
        if log_sync {
            self.append_sync_event(
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Created,
                serde_json::json!({ "entity": &task }),
            )?;
        }
        Ok(task)
    }

    pub fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task> {
        let mut task = self.get_task(id)?;
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
        self.save_task(&task)?;
        self.append_sync_event(
            SyncEntityType::Task,
            &task.id,
            SyncOperation::Updated,
            serde_json::json!({ "entity": &task }),
        )?;
        Ok(task)
    }

    pub fn transition_task(
        &self,
        id: &str,
        status: TaskStatus,
        blocked_reason: Option<String>,
    ) -> Result<Task> {
        self.transition_task_internal(id, status, blocked_reason, true)
    }

    fn transition_task_internal(
        &self,
        id: &str,
        status: TaskStatus,
        blocked_reason: Option<String>,
        log_sync: bool,
    ) -> Result<Task> {
        let mut task = self.get_task(id)?;
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
        self.save_task(&task)?;
        if log_sync {
            self.append_sync_event(
                SyncEntityType::Task,
                &task.id,
                SyncOperation::Transitioned,
                serde_json::json!({ "entity": &task }),
            )?;
        }
        Ok(task)
    }

    fn save_task(&self, task: &Task) -> Result<()> {
        let status = task.status.to_string();
        let due_at = task.due_at.map(timestamp);
        let scheduled_at = task.scheduled_at.map(timestamp);
        let started_at = task.started_at.map(timestamp);
        let completed_at = task.completed_at.map(timestamp);
        let tags = serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".into());
        let created_at = timestamp(task.created_at);
        let updated_at = timestamp(task.updated_at);
        self.connection()?.execute(
            "UPDATE tasks SET project_id=?2,title=?3,description=?4,status=?5,priority=?6,due_at=?7,
             scheduled_at=?8,started_at=?9,completed_at=?10,estimated_minutes=?11,
             time_limit_minutes=?12,pinned=?13,blocked_reason=?14,tags=?15,created_at=?16,
             updated_at=?17 WHERE id=?1",
            params![
                task.id, task.project_id, task.title, task.description, status, task.priority,
                due_at, scheduled_at, started_at, completed_at, task.estimated_minutes,
                task.time_limit_minutes, task.pinned, task.blocked_reason, tags, created_at,
                updated_at
            ],
        )?;
        Ok(())
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
                    false,
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
                false,
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
                    false,
                )?;
                if let Some(reason) = blocked_reason {
                    self.transition_task_internal(
                        &task.id,
                        TaskStatus::Blocked,
                        Some(reason.into()),
                        false,
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
    let payload_json = row.get::<_, String>(6)?;
    Ok(SyncEvent {
        event_id: row.get(0)?,
        device_id: row.get(1)?,
        sequence: row.get(2)?,
        entity_type: parse_enum(row.get::<_, String>(3)?)?,
        entity_id: row.get(4)?,
        operation: parse_enum(row.get::<_, String>(5)?)?,
        payload_json: serde_json::from_str(&payload_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        created_at: parse_time(row.get(7)?)?,
        synced_at: parse_optional_time(row.get(8)?)?,
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
    use crate::sync::{SyncEntityType, SyncOperation};

    fn seeded_database() -> Database {
        let db = Database::in_memory().unwrap();
        db.seed().unwrap();
        db
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
        for table in ["sync_events", "sync_state", "sync_devices"] {
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
