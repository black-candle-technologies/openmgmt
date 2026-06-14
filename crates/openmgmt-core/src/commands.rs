use crate::{
    db::{Database, Result},
    models::{
        BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch, Project,
        ProjectPatch, Task, TaskPatch, TaskStatus,
    },
    sync::{SyncSettings, SyncSettingsPatch, SyncStatus},
};

#[derive(Clone)]
pub struct AppService {
    database: Database,
}

impl AppService {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn database(&self) -> Database {
        self.database.clone()
    }

    pub fn list_organizations(&self) -> Result<Vec<Organization>> {
        self.database.list_organizations()
    }
    pub fn create_organization(&self, input: NewOrganization) -> Result<Organization> {
        self.database.create_organization(input)
    }
    pub fn update_organization(&self, id: &str, patch: OrganizationPatch) -> Result<Organization> {
        self.database.update_organization(id, patch)
    }
    pub fn archive_organization(&self, id: &str) -> Result<()> {
        self.database.archive_organization(id)
    }
    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.database.list_projects()
    }
    pub fn create_project(&self, input: NewProject) -> Result<Project> {
        self.database.create_project(input)
    }
    pub fn get_project(&self, id: &str) -> Result<Project> {
        self.database.get_project(id)
    }
    pub fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project> {
        self.database.update_project(id, patch)
    }
    pub fn archive_project(&self, id: &str) -> Result<()> {
        self.database.archive_project(id)
    }
    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        self.database.list_tasks()
    }
    pub fn create_task(&self, input: NewTask) -> Result<Task> {
        self.database.create_task(input)
    }
    pub fn get_task(&self, id: &str) -> Result<Task> {
        self.database.get_task(id)
    }
    pub fn update_task(&self, id: &str, patch: TaskPatch) -> Result<Task> {
        self.database.update_task(id, patch)
    }
    pub fn cancel_task(&self, id: &str) -> Result<Task> {
        self.database
            .transition_task(id, TaskStatus::Canceled, None)
    }
    pub fn start_task(&self, id: &str) -> Result<Task> {
        self.database
            .transition_task(id, TaskStatus::InProgress, None)
    }
    pub fn complete_task(&self, id: &str) -> Result<Task> {
        self.database.transition_task(id, TaskStatus::Done, None)
    }
    pub fn block_task(&self, id: &str, reason: String) -> Result<Task> {
        self.database
            .transition_task(id, TaskStatus::Blocked, Some(reason))
    }
    pub fn unblock_task(&self, id: &str) -> Result<Task> {
        self.database.transition_task(id, TaskStatus::Ready, None)
    }
    pub fn get_board_state(&self) -> Result<BoardState> {
        self.database.board_state()
    }
    pub fn seed_database(&self) -> Result<()> {
        self.database.seed()
    }
    pub fn get_sync_settings(&self) -> Result<SyncSettings> {
        self.database.get_sync_settings()
    }
    pub fn update_sync_settings(&self, patch: SyncSettingsPatch) -> Result<SyncSettings> {
        self.database.update_sync_settings(patch)
    }
    pub fn get_sync_status(&self) -> Result<SyncStatus> {
        self.database.get_sync_status()
    }
    pub fn record_sync_attempt_started(&self) -> Result<SyncStatus> {
        self.database.record_sync_attempt_started()
    }
    pub fn record_sync_success(&self) -> Result<SyncStatus> {
        self.database.record_sync_success()
    }
    pub fn record_sync_error(&self, error: &str) -> Result<SyncStatus> {
        self.database.record_sync_error(error)
    }
    pub fn clear_sync_error(&self) -> Result<SyncStatus> {
        self.database.clear_sync_error()
    }
}
