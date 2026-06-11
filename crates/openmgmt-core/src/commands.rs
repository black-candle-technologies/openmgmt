use crate::{
    db::{Database, Result},
    models::{
        BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch, Project,
        ProjectPatch, Task, TaskPatch, TaskStatus,
    },
};

#[derive(Clone)]
pub struct AppService {
    database: Database,
}

impl AppService {
    pub fn new(database: Database) -> Self {
        Self { database }
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
}
