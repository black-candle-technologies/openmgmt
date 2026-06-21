use crate::{
    db::{Database, Result},
    local_ai::{
        OllamaClient, plan_day_prompt, prompt_messages, rewrite_task_prompt,
        suggest_next_task_prompt, summarize_project_prompt, triage_tasks_prompt, workflow_error,
        workflow_response,
    },
    models::{
        BoardState, CalendarBlock, LocalAiChatMessage, LocalAiChatResponse,
        LocalAiConnectionResult, LocalAiModelListResult, LocalAiSettings, LocalAiSettingsPatch,
        LocalAiWorkflowResponse, NewOrganization, NewProject, NewSavedTaskView, NewTask,
        Organization, OrganizationPatch, Project, ProjectPatch, SavedTaskView, SavedTaskViewPatch,
        ScheduleConflict, ScheduleTaskInput, ScheduledBlockCompletion, ScoringSettings,
        ScoringSettingsPatch, Task, TaskPatch, TaskQueryFilter, TaskSort, TaskStatus,
        TaskTimerSession, TaskWithContext, TimeBlockSuggestion,
    },
    sync::{SyncSettings, SyncSettingsPatch, SyncStatus},
};
use chrono::{DateTime, Duration, Utc};

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
    pub fn start_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        self.database.start_task_timer(task_id)
    }
    pub fn pause_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        self.database.pause_task_timer(task_id)
    }
    pub fn resume_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        self.database.resume_task_timer(task_id)
    }
    pub fn stop_task_timer(&self, task_id: &str) -> Result<TaskTimerSession> {
        self.database.stop_task_timer(task_id)
    }
    pub fn complete_task_with_timer(&self, task_id: &str) -> Result<Task> {
        self.database.complete_task_with_timer(task_id)
    }
    pub fn list_task_timer_sessions(&self, task_id: &str) -> Result<Vec<TaskTimerSession>> {
        self.database.list_task_timer_sessions(task_id)
    }
    pub fn get_active_timer_session(&self, task_id: &str) -> Result<Option<TaskTimerSession>> {
        self.database.get_active_timer_session(task_id)
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
    pub fn get_schedule_today(&self) -> Result<Vec<TaskWithContext>> {
        self.database.get_schedule_today()
    }
    pub fn get_schedule_week(&self) -> Result<Vec<TaskWithContext>> {
        self.database.get_schedule_week()
    }
    pub fn get_schedule_for_day(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<TaskWithContext>> {
        self.database.get_schedule_for_day(start, end)
    }
    pub fn get_unscheduled_tasks(&self) -> Result<Vec<TaskWithContext>> {
        self.database.get_unscheduled_tasks()
    }
    pub fn get_overdue_tasks(&self) -> Result<Vec<TaskWithContext>> {
        self.database.get_overdue_tasks()
    }
    pub fn auto_start_due_scheduled_tasks(&self) -> Result<Vec<Task>> {
        self.database.auto_start_due_scheduled_tasks()
    }
    pub fn schedule_task(&self, task_id: &str, input: ScheduleTaskInput) -> Result<CalendarBlock> {
        self.database.schedule_task(task_id, input)
    }
    pub fn reschedule_task(
        &self,
        task_id: &str,
        input: ScheduleTaskInput,
    ) -> Result<CalendarBlock> {
        self.database.reschedule_task(task_id, input)
    }
    pub fn clear_task_schedule(&self, task_id: &str) -> Result<Task> {
        self.database.clear_task_schedule(task_id)
    }
    pub fn list_schedule_conflicts(&self) -> Result<Vec<ScheduleConflict>> {
        self.database.list_schedule_conflicts()
    }
    pub fn suggest_next_time_block(
        &self,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        duration_minutes: i64,
    ) -> Result<Option<TimeBlockSuggestion>> {
        self.database
            .suggest_next_time_block(window_start, window_end, duration_minutes)
    }
    pub fn suggest_tasks_for_time_window(
        &self,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<Vec<TaskWithContext>> {
        self.database
            .suggest_tasks_for_time_window(window_start, window_end)
    }
    pub fn complete_scheduled_block(&self, block_id: &str) -> Result<ScheduledBlockCompletion> {
        self.database.complete_scheduled_block(block_id)
    }
    pub fn skip_scheduled_block(&self, block_id: &str) -> Result<CalendarBlock> {
        self.database.skip_scheduled_block(block_id)
    }
    pub fn generate_schedule_ics(&self) -> Result<String> {
        self.database.generate_schedule_ics()
    }
    pub fn list_saved_task_views(&self) -> Result<Vec<SavedTaskView>> {
        self.database.list_saved_task_views()
    }
    pub fn get_saved_task_view(&self, id: &str) -> Result<SavedTaskView> {
        self.database.get_saved_task_view(id)
    }
    pub fn create_saved_task_view(&self, input: NewSavedTaskView) -> Result<SavedTaskView> {
        self.database.create_saved_task_view(input)
    }
    pub fn update_saved_task_view(
        &self,
        id: &str,
        patch: SavedTaskViewPatch,
    ) -> Result<SavedTaskView> {
        self.database.update_saved_task_view(id, patch)
    }
    pub fn archive_saved_task_view(&self, id: &str) -> Result<()> {
        self.database.archive_saved_task_view(id)
    }
    pub fn query_tasks(
        &self,
        filter: TaskQueryFilter,
        sort: Option<TaskSort>,
    ) -> Result<Vec<TaskWithContext>> {
        self.database.query_tasks(filter, sort)
    }
    pub fn get_scoring_settings(&self) -> Result<ScoringSettings> {
        self.database.get_scoring_settings()
    }
    pub fn update_scoring_settings(&self, patch: ScoringSettingsPatch) -> Result<ScoringSettings> {
        self.database.update_scoring_settings(patch)
    }
    pub fn reset_scoring_settings(&self) -> Result<ScoringSettings> {
        self.database.reset_scoring_settings()
    }
    pub fn get_local_ai_settings(&self) -> Result<LocalAiSettings> {
        self.database.get_local_ai_settings()
    }
    pub fn update_local_ai_settings(&self, patch: LocalAiSettingsPatch) -> Result<LocalAiSettings> {
        self.database.update_local_ai_settings(patch)
    }
    pub fn reset_local_ai_settings(&self) -> Result<LocalAiSettings> {
        self.database.reset_local_ai_settings()
    }
    pub async fn test_ollama_connection(&self) -> Result<LocalAiConnectionResult> {
        let settings = self.database.get_local_ai_settings()?;
        let result = OllamaClient::new(&settings, 4)?.test_connection().await;
        let _ = self
            .database
            .update_local_ai_connection_status(result.connected, result.error.clone())?;
        Ok(result)
    }
    pub async fn list_ollama_models(&self) -> Result<LocalAiModelListResult> {
        let settings = self.database.get_local_ai_settings()?;
        Ok(OllamaClient::new(&settings, 5)?.list_models().await)
    }
    pub async fn run_ollama_chat(
        &self,
        model: Option<String>,
        messages: Vec<LocalAiChatMessage>,
    ) -> Result<LocalAiChatResponse> {
        let settings = self.database.get_local_ai_settings()?;
        if !settings.enabled {
            return Err(crate::db::CoreError::Validation(
                "local AI integration is disabled".into(),
            ));
        }
        OllamaClient::new(&settings, 30)?
            .chat(&settings, model, messages)
            .await
    }
    pub async fn plan_day_with_ollama(&self) -> Result<LocalAiWorkflowResponse> {
        let board = self.database.board_state()?;
        let schedule = self.database.get_schedule_today()?;
        Ok(
            match self
                .run_ollama_chat(None, prompt_messages(plan_day_prompt(&board, &schedule)))
                .await
            {
                Ok(chat) => workflow_response(chat, false, None),
                Err(error) => workflow_error(error),
            },
        )
    }
    pub async fn summarize_project_with_ollama(
        &self,
        project_id: &str,
    ) -> Result<LocalAiWorkflowResponse> {
        let project = self.database.get_project(project_id)?;
        let tasks = self
            .database
            .list_tasks()?
            .into_iter()
            .filter(|task| task.project_id == project_id)
            .collect::<Vec<_>>();
        Ok(
            match self
                .run_ollama_chat(
                    None,
                    prompt_messages(summarize_project_prompt(&project, &tasks)),
                )
                .await
            {
                Ok(chat) => workflow_response(chat, false, None),
                Err(error) => workflow_error(error),
            },
        )
    }
    pub async fn triage_tasks_with_ollama(&self) -> Result<LocalAiWorkflowResponse> {
        let now = Utc::now();
        let overdue = self.database.get_overdue_tasks()?;
        let unscheduled = self.database.get_unscheduled_tasks()?;
        let tasks = self
            .database
            .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?;
        let blocked = tasks
            .iter()
            .filter(|item| matches!(item.task.status, TaskStatus::Blocked | TaskStatus::Waiting))
            .cloned()
            .collect::<Vec<_>>();
        let due_soon = tasks
            .iter()
            .filter(|item| {
                item.task
                    .due_at
                    .is_some_and(|due| due >= now && due <= now + Duration::hours(24))
            })
            .cloned()
            .collect::<Vec<_>>();
        Ok(
            match self
                .run_ollama_chat(
                    None,
                    prompt_messages(triage_tasks_prompt(
                        &overdue,
                        &blocked,
                        &due_soon,
                        &unscheduled,
                    )),
                )
                .await
            {
                Ok(chat) => workflow_response(chat, false, None),
                Err(error) => workflow_error(error),
            },
        )
    }
    pub async fn suggest_next_task_with_ollama(&self) -> Result<LocalAiWorkflowResponse> {
        let board = self.database.board_state()?;
        let tasks = self
            .database
            .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?;
        match self
            .run_ollama_chat(
                None,
                prompt_messages(suggest_next_task_prompt(&board, &tasks)),
            )
            .await
        {
            Ok(chat) => Ok(workflow_response(chat, false, None)),
            Err(error) => Ok(LocalAiWorkflowResponse {
                content: tasks
                    .first()
                    .map(|task| {
                        format!(
                            "Fallback next task: P{} {}. {}",
                            task.task.priority, task.task.title, error
                        )
                    })
                    .unwrap_or_else(|| format!("No available task. {error}")),
                model: None,
                fallback_used: true,
                fallback_task: tasks.first().cloned(),
                error: Some(error.to_string()),
            }),
        }
    }
    pub async fn rewrite_task_description_with_ollama(
        &self,
        task_id: &str,
        instruction: &str,
    ) -> Result<LocalAiWorkflowResponse> {
        let task = self.database.get_task(task_id)?;
        Ok(
            match self
                .run_ollama_chat(
                    None,
                    prompt_messages(rewrite_task_prompt(&task, instruction)),
                )
                .await
            {
                Ok(chat) => workflow_response(chat, false, None),
                Err(error) => workflow_error(error),
            },
        )
    }
    pub fn export_tasks_json(&self) -> Result<String> {
        self.database.export_tasks_json()
    }
    pub fn export_tasks_csv(&self) -> Result<String> {
        self.database.export_tasks_csv()
    }
    pub fn export_all_json(&self) -> Result<String> {
        self.database.export_all_json()
    }
    pub fn backup_sqlite_database(&self, target_path: &str) -> Result<()> {
        self.database.backup_sqlite_database(target_path)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_ai::{DEFAULT_OLLAMA_MODEL, prompt_messages};
    use crate::models::{LocalAiSettingsPatch, NewOrganization, NewProject, NewTask, ProjectType};

    #[tokio::test]
    async fn suggest_next_task_falls_back_when_ollama_is_unavailable() {
        let database = Database::in_memory().unwrap();
        database
            .update_local_ai_settings(LocalAiSettingsPatch {
                base_url: Some("http://127.0.0.1:9".into()),
                ..Default::default()
            })
            .unwrap();
        let organization = database
            .create_organization(NewOrganization {
                name: "Test".into(),
                slug: None,
                description: None,
                color: None,
                icon: None,
            })
            .unwrap();
        let project = database
            .create_project(NewProject {
                organization_id: organization.id,
                name: "Project".into(),
                slug: None,
                description: None,
                project_type: ProjectType::Software,
                status: Default::default(),
                priority: 3,
                deadline: None,
                repo_url: None,
                notes: None,
            })
            .unwrap();
        let p1 = database
            .create_task(NewTask {
                project_id: project.id,
                title: "Top task".into(),
                description: None,
                status: Default::default(),
                priority: 1,
                due_at: None,
                scheduled_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                tags: Vec::new(),
            })
            .unwrap();

        let response = AppService::new(database)
            .suggest_next_task_with_ollama()
            .await
            .unwrap();

        assert!(response.fallback_used);
        assert_eq!(response.fallback_task.unwrap().task.id, p1.id);
    }

    #[tokio::test]
    #[ignore]
    async fn live_ollama_smoke() {
        let service = AppService::new(Database::in_memory().unwrap());

        assert!(service.test_ollama_connection().await.unwrap().connected);
        assert!(service.list_ollama_models().await.unwrap().connected);
        assert!(
            !service
                .run_ollama_chat(
                    Some(DEFAULT_OLLAMA_MODEL.into()),
                    prompt_messages("Say ready.".into()),
                )
                .await
                .unwrap()
                .content
                .trim()
                .is_empty()
        );
        assert!(
            service
                .plan_day_with_ollama()
                .await
                .unwrap()
                .error
                .is_none()
        );
    }
}
