use crate::{
    db::{Database, Result},
    local_ai::{
        OllamaClient, SlashCommand, chat_prompt_messages, compact_task_lines, local_ai_tool,
        local_ai_tool_registry, parse_slash_command, plan_day_prompt, prompt_messages,
        rewrite_task_prompt, slash_help, suggest_next_task_prompt, summarize_project_prompt,
        triage_tasks_prompt, workflow_error, workflow_response,
    },
    models::{
        BoardState, CalendarBlock, LocalAiChatMessage, LocalAiChatMessageRecord,
        LocalAiChatResponse, LocalAiChatRole, LocalAiChatSession, LocalAiChatTurn,
        LocalAiConnectionResult, LocalAiContextScope, LocalAiModelListResult, LocalAiSettings,
        LocalAiSettingsPatch, LocalAiToolCall, LocalAiToolCallStatus, LocalAiToolDefinition,
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
    pub fn list_local_ai_chat_sessions(&self) -> Result<Vec<LocalAiChatSession>> {
        self.database.list_local_ai_chat_sessions()
    }
    pub fn create_local_ai_chat_session(
        &self,
        title: Option<String>,
        model: Option<String>,
    ) -> Result<LocalAiChatSession> {
        self.database.create_local_ai_chat_session(title, model)
    }
    pub fn archive_local_ai_chat_session(&self, id: &str) -> Result<LocalAiChatSession> {
        self.database.archive_local_ai_chat_session(id)
    }
    pub fn list_local_ai_chat_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<LocalAiChatMessageRecord>> {
        self.database.list_local_ai_chat_messages(session_id)
    }
    pub fn list_local_ai_tools(&self) -> Vec<LocalAiToolDefinition> {
        local_ai_tool_registry()
    }
    pub fn build_local_ai_chat_context(&self, scope: LocalAiContextScope) -> Result<String> {
        let organizations = self.database.list_organizations()?;
        let projects = self.database.list_projects()?;
        let tasks = self
            .database
            .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?;
        let board = self.database.board_state()?;
        let mut lines = vec![
            "OpenMgmt local context.".to_string(),
            "P1 is highest priority; P5 is lowest.".to_string(),
            format!(
                "Workspace counts: {} organizations, {} projects, {} active tasks.",
                organizations.len(),
                projects.len(),
                tasks.len()
            ),
        ];
        if matches!(
            scope,
            LocalAiContextScope::Daily
                | LocalAiContextScope::FullSummary
                | LocalAiContextScope::Minimal
        ) {
            lines.push(format!(
                "Board now:\n{}",
                compact_task_lines(
                    &board
                        .now
                        .iter()
                        .map(|item| scored_to_context(item, &tasks))
                        .collect::<Vec<_>>(),
                    8
                )
            ));
            lines.push(format!(
                "Overdue:\n{}",
                compact_task_lines(&self.database.get_overdue_tasks()?, 8)
            ));
            lines.push(format!(
                "Blocked/waiting:\n{}",
                compact_task_lines(
                    &tasks
                        .iter()
                        .filter(|item| {
                            matches!(item.task.status, TaskStatus::Blocked | TaskStatus::Waiting)
                        })
                        .cloned()
                        .collect::<Vec<_>>(),
                    8
                )
            ));
        }
        if matches!(
            scope,
            LocalAiContextScope::Schedule
                | LocalAiContextScope::Daily
                | LocalAiContextScope::FullSummary
        ) {
            lines.push(format!(
                "Schedule today:\n{}",
                compact_task_lines(&self.database.get_schedule_today()?, 8)
            ));
            lines.push(format!(
                "Schedule week:\n{}",
                compact_task_lines(&self.database.get_schedule_week()?, 8)
            ));
            lines.push(format!(
                "Unscheduled:\n{}",
                compact_task_lines(&self.database.get_unscheduled_tasks()?, 8)
            ));
        }
        Ok(lines.join("\n\n"))
    }
    pub async fn send_local_ai_chat_message(
        &self,
        input: crate::models::SendLocalAiChatMessageInput,
    ) -> Result<LocalAiChatTurn> {
        let mut session = if let Some(id) = input.session_id.as_deref() {
            self.database.get_local_ai_chat_session(id)?
        } else {
            self.database.create_local_ai_chat_session(
                Some(chat_title(&input.message)),
                input.model.clone(),
            )?
        };
        if let Some(model) = input.model.clone() {
            session = self
                .database
                .update_local_ai_chat_session_model(&session.id, Some(model))?;
        }
        let user = self.database.append_local_ai_chat_message(
            &session.id,
            LocalAiChatRole::User,
            input.message.clone(),
            session.model.clone(),
            None,
        )?;
        if let Some(command) = parse_slash_command(&input.message)? {
            self.handle_local_ai_slash_command(&mut session, &user, command)
                .await?;
            return self.local_ai_chat_turn(session);
        }
        let context = self.build_local_ai_chat_context(input.context_scope.unwrap_or_default())?;
        let messages = chat_prompt_messages(context, input.message);
        let output = match self.run_ollama_chat(session.model.clone(), messages).await {
            Ok(chat) => chat.content,
            Err(error) => format!("Local AI unavailable: {error}"),
        };
        self.database.append_local_ai_chat_message(
            &session.id,
            LocalAiChatRole::Assistant,
            output,
            session.model.clone(),
            None,
        )?;
        self.local_ai_chat_turn(session)
    }
    pub async fn run_local_ai_slash_command(
        &self,
        session_id: Option<String>,
        command: String,
    ) -> Result<LocalAiChatTurn> {
        self.send_local_ai_chat_message(crate::models::SendLocalAiChatMessageInput {
            session_id,
            message: command,
            model: None,
            context_scope: Some(LocalAiContextScope::Daily),
            allow_write_proposals: true,
        })
        .await
    }
    pub fn confirm_local_ai_tool_call(&self, id: &str) -> Result<LocalAiToolCall> {
        let mut call = self.database.get_local_ai_tool_call(id)?;
        if call.status != LocalAiToolCallStatus::Proposed {
            return Err(crate::db::CoreError::Validation(
                "only proposed tool calls can be confirmed".into(),
            ));
        }
        call.status = LocalAiToolCallStatus::Confirmed;
        call.updated_at = Utc::now();
        self.database.save_local_ai_tool_call(&call)?;
        Ok(call)
    }
    pub fn cancel_local_ai_tool_call(&self, id: &str) -> Result<LocalAiToolCall> {
        let mut call = self.database.get_local_ai_tool_call(id)?;
        if call.status == LocalAiToolCallStatus::Executed {
            return Err(crate::db::CoreError::Validation(
                "executed tool calls cannot be canceled".into(),
            ));
        }
        call.status = LocalAiToolCallStatus::Canceled;
        call.updated_at = Utc::now();
        self.database.save_local_ai_tool_call(&call)?;
        Ok(call)
    }
    pub fn execute_local_ai_tool_call(&self, id: &str) -> Result<LocalAiToolCall> {
        let mut call = self.database.get_local_ai_tool_call(id)?;
        let tool = local_ai_tool(&call.tool_name)
            .ok_or(crate::db::CoreError::NotFound("local AI tool"))?;
        if tool.write && call.status != LocalAiToolCallStatus::Confirmed {
            return Err(crate::db::CoreError::Validation(
                "write tool calls require confirmation before execution".into(),
            ));
        }
        match self.execute_known_local_ai_tool(&call) {
            Ok(result) => {
                call.result_json = Some(result.clone());
                call.status = LocalAiToolCallStatus::Executed;
                call.error_message = None;
                call.updated_at = Utc::now();
                self.database.save_local_ai_tool_call(&call)?;
                let _ = self.database.append_local_ai_chat_message(
                    &call.session_id,
                    LocalAiChatRole::Tool,
                    result.to_string(),
                    None,
                    Some(serde_json::json!({"tool_call_id": call.id})),
                )?;
                Ok(call)
            }
            Err(error) => {
                call.status = LocalAiToolCallStatus::Failed;
                call.error_message = Some(error.to_string());
                call.updated_at = Utc::now();
                self.database.save_local_ai_tool_call(&call)?;
                Err(error)
            }
        }
    }
    async fn handle_local_ai_slash_command(
        &self,
        session: &mut LocalAiChatSession,
        user_message: &LocalAiChatMessageRecord,
        command: SlashCommand,
    ) -> Result<()> {
        let output = match command {
            SlashCommand::Help => slash_help(),
            SlashCommand::Plan => self.plan_day_with_ollama().await?.content,
            SlashCommand::Board => serde_json::to_string_pretty(&self.database.board_state()?)
                .map_err(|error| crate::db::CoreError::Validation(error.to_string()))?,
            SlashCommand::Tasks(filter) => {
                let mut tasks = self
                    .database
                    .query_tasks(TaskQueryFilter::default(), Some(TaskSort::default()))?;
                if let Some(filter) = filter.as_deref() {
                    tasks.retain(|item| match filter {
                        "blocked" => {
                            matches!(item.task.status, TaskStatus::Blocked | TaskStatus::Waiting)
                        }
                        "overdue" => item.task.due_at.is_some_and(|due| due < Utc::now()),
                        _ => true,
                    });
                }
                compact_task_lines(&tasks, 30)
            }
            SlashCommand::ScheduleToday => {
                compact_task_lines(&self.database.get_schedule_today()?, 30)
            }
            SlashCommand::ScheduleWeek => {
                compact_task_lines(&self.database.get_schedule_week()?, 30)
            }
            SlashCommand::Unscheduled => {
                compact_task_lines(&self.database.get_unscheduled_tasks()?, 30)
            }
            SlashCommand::Models => {
                let result = self.list_ollama_models().await?;
                if !result.connected {
                    result.error.unwrap_or_else(|| "Ollama unavailable".into())
                } else if result.models.is_empty() {
                    "No local Ollama models installed.".into()
                } else {
                    result
                        .models
                        .into_iter()
                        .map(|model| model.name)
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            SlashCommand::UseModel(model) => {
                *session = self
                    .database
                    .update_local_ai_chat_session_model(&session.id, Some(model.clone()))?;
                format!("Using model `{model}` for this session.")
            }
            SlashCommand::CreateTask(title) => {
                self.database.create_local_ai_tool_call(
                    &session.id,
                    Some(user_message.id.clone()),
                    "create_task".into(),
                    serde_json::json!({ "title": title }),
                    LocalAiToolCallStatus::Proposed,
                )?;
                "Proposed `create_task`. Confirm before execution.".into()
            }
            SlashCommand::CompleteTask(id) => {
                self.database.create_local_ai_tool_call(
                    &session.id,
                    Some(user_message.id.clone()),
                    "complete_task".into(),
                    serde_json::json!({ "task_id": id }),
                    LocalAiToolCallStatus::Proposed,
                )?;
                "Proposed `complete_task`. Confirm before execution.".into()
            }
            SlashCommand::StartTask(id) => {
                self.database.create_local_ai_tool_call(
                    &session.id,
                    Some(user_message.id.clone()),
                    "start_task".into(),
                    serde_json::json!({ "task_id": id }),
                    LocalAiToolCallStatus::Proposed,
                )?;
                "Proposed `start_task`. Confirm before execution.".into()
            }
        };
        self.database.append_local_ai_chat_message(
            &session.id,
            LocalAiChatRole::Assistant,
            output,
            session.model.clone(),
            None,
        )?;
        Ok(())
    }

    fn local_ai_chat_turn(&self, session: LocalAiChatSession) -> Result<LocalAiChatTurn> {
        let messages = self.database.list_local_ai_chat_messages(&session.id)?;
        let proposed_tool_calls = self
            .database
            .list_local_ai_tool_calls(&session.id)?
            .into_iter()
            .filter(|call| {
                matches!(
                    call.status,
                    LocalAiToolCallStatus::Proposed | LocalAiToolCallStatus::Confirmed
                )
            })
            .collect();
        let assistant_output = messages
            .iter()
            .rev()
            .find(|message| message.role == LocalAiChatRole::Assistant)
            .map(|message| message.content.clone());
        Ok(LocalAiChatTurn {
            session,
            messages,
            proposed_tool_calls,
            assistant_output,
        })
    }

    fn execute_known_local_ai_tool(&self, call: &LocalAiToolCall) -> Result<serde_json::Value> {
        let args = &call.arguments_json;
        let result = match call.tool_name.as_str() {
            "get_summary" => serde_json::json!({
                "organizations": self.database.list_organizations()?.len(),
                "projects": self.database.list_projects()?.len(),
                "tasks": self.database.list_tasks()?.len(),
            }),
            "get_board" => serde_json::to_value(self.database.board_state()?)?,
            "get_daily_operations" => serde_json::json!({
                "board": self.database.board_state()?,
                "overdue": self.database.get_overdue_tasks()?,
                "unscheduled": self.database.get_unscheduled_tasks()?,
            }),
            "get_schedule_day" => serde_json::to_value(self.database.get_schedule_today()?)?,
            "get_schedule_week" => serde_json::to_value(self.database.get_schedule_week()?)?,
            "list_tasks" => serde_json::to_value(self.database.list_tasks()?)?,
            "list_projects" => serde_json::to_value(self.database.list_projects()?)?,
            "list_organizations" => serde_json::to_value(self.database.list_organizations()?)?,
            "get_task" => {
                serde_json::to_value(self.database.get_task(required_arg(args, "task_id")?)?)?
            }
            "get_project" => serde_json::to_value(
                self.database
                    .get_project(required_arg(args, "project_id")?)?,
            )?,
            "create_organization" => {
                serde_json::to_value(self.database.create_organization(NewOrganization {
                    name: required_arg(args, "name")?.into(),
                    slug: None,
                    description: optional_arg(args, "description").map(str::to_owned),
                    color: None,
                    icon: None,
                })?)?
            }
            "create_project" => serde_json::to_value(
                self.database.create_project(NewProject {
                    organization_id: optional_arg(args, "organization_id")
                        .map(str::to_owned)
                        .or_else(|| {
                            self.database
                                .list_organizations()
                                .ok()?
                                .first()
                                .map(|org| org.id.clone())
                        })
                        .ok_or(crate::db::CoreError::Validation(
                            "organization_id is required".into(),
                        ))?,
                    name: required_arg(args, "name")?.into(),
                    slug: None,
                    description: optional_arg(args, "description").map(str::to_owned),
                    project_type: Default::default(),
                    status: Default::default(),
                    priority: 3,
                    deadline: None,
                    repo_url: None,
                    notes: None,
                })?,
            )?,
            "create_task" => serde_json::to_value(
                self.database.create_task(NewTask {
                    project_id: optional_arg(args, "project_id")
                        .map(str::to_owned)
                        .or_else(|| {
                            self.database
                                .list_projects()
                                .ok()?
                                .first()
                                .map(|project| project.id.clone())
                        })
                        .ok_or(crate::db::CoreError::Validation(
                            "project_id is required".into(),
                        ))?,
                    title: required_arg(args, "title")?.into(),
                    description: optional_arg(args, "description").map(str::to_owned),
                    status: Default::default(),
                    priority: optional_i32_arg(args, "priority").unwrap_or(3),
                    due_at: None,
                    scheduled_at: None,
                    estimated_minutes: None,
                    time_limit_minutes: None,
                    pinned: false,
                    tags: Vec::new(),
                })?,
            )?,
            "update_task" => serde_json::to_value(
                self.database.update_task(
                    required_arg(args, "task_id")?,
                    TaskPatch {
                        title: optional_arg(args, "title").map(str::to_owned),
                        description: args
                            .get("description")
                            .and_then(|value| value.as_str())
                            .map(|value| Some(value.to_owned())),
                        status: None,
                        priority: optional_i32_arg(args, "priority"),
                        ..Default::default()
                    },
                )?,
            )?,
            "schedule_task" => serde_json::to_value(self.database.schedule_task(
                required_arg(args, "task_id")?,
                schedule_input_from_args(args)?,
            )?)?,
            "reschedule_task" => serde_json::to_value(self.database.reschedule_task(
                required_arg(args, "task_id")?,
                schedule_input_from_args(args)?,
            )?)?,
            "start_task" => serde_json::to_value(self.database.transition_task(
                required_arg(args, "task_id")?,
                TaskStatus::InProgress,
                None,
            )?)?,
            "block_task" => serde_json::to_value(
                self.database.transition_task(
                    required_arg(args, "task_id")?,
                    TaskStatus::Blocked,
                    Some(
                        optional_arg(args, "reason")
                            .unwrap_or("Blocked by local AI proposal")
                            .into(),
                    ),
                )?,
            )?,
            "complete_task" => serde_json::to_value(self.database.transition_task(
                required_arg(args, "task_id")?,
                TaskStatus::Done,
                None,
            )?)?,
            _ => return Err(crate::db::CoreError::NotFound("local AI tool")),
        };
        Ok(result)
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

fn chat_title(message: &str) -> String {
    let title = message.trim().chars().take(48).collect::<String>();
    if title.is_empty() {
        "Local AI chat".into()
    } else {
        title
    }
}

fn scored_to_context(
    scored: &crate::models::ScoredTask,
    tasks: &[TaskWithContext],
) -> TaskWithContext {
    tasks
        .iter()
        .find(|item| item.task.id == scored.context.task.id)
        .cloned()
        .unwrap_or_else(|| TaskWithContext {
            task: scored.context.task.clone(),
            project_id: String::new(),
            project_name: scored.context.project_name.clone(),
            project_type: scored.context.project_type,
            organization_id: String::new(),
            organization_name: scored.context.organization_name.clone(),
            organization_color: scored.context.organization_color.clone(),
            organization_icon: None,
            urgency_score: scored.urgency_score,
            active_timer: None,
        })
}

fn required_arg<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    optional_arg(args, key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| crate::db::CoreError::Validation(format!("{key} is required")))
}

fn optional_arg<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|value| value.as_str())
}

fn optional_i32_arg(args: &serde_json::Value, key: &str) -> Option<i32> {
    args.get(key)
        .and_then(|value| value.as_i64())
        .and_then(|value| i32::try_from(value).ok())
}

fn schedule_input_from_args(args: &serde_json::Value) -> Result<ScheduleTaskInput> {
    Ok(ScheduleTaskInput {
        start_at: DateTime::parse_from_rfc3339(required_arg(args, "start_at")?)
            .map_err(|error| crate::db::CoreError::Validation(error.to_string()))?
            .with_timezone(&Utc),
        end_at: DateTime::parse_from_rfc3339(required_arg(args, "end_at")?)
            .map_err(|error| crate::db::CoreError::Validation(error.to_string()))?
            .with_timezone(&Utc),
        timezone: optional_arg(args, "timezone").map(str::to_owned),
        reminder_at: None,
        deadline_at: None,
        recurrence_rule: None,
        recurrence_anchor_at: None,
        recurrence_timezone: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_ai::{DEFAULT_OLLAMA_MODEL, prompt_messages};
    use crate::models::{
        LocalAiSettingsPatch, NewOrganization, NewProject, NewTask, ProjectType,
        SendLocalAiChatMessageInput,
    };

    fn service_with_project() -> (AppService, Project) {
        let database = Database::in_memory().unwrap();
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
        (AppService::new(database), project)
    }

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
    async fn slash_help_and_board_return_messages() {
        let service = AppService::new(Database::in_memory().unwrap());
        let help = service
            .send_local_ai_chat_message(SendLocalAiChatMessageInput {
                session_id: None,
                message: "/help".into(),
                model: None,
                context_scope: None,
                allow_write_proposals: true,
            })
            .await
            .unwrap();
        assert!(help.assistant_output.unwrap().contains("/plan"));

        let board = service
            .send_local_ai_chat_message(SendLocalAiChatMessageInput {
                session_id: Some(help.session.id),
                message: "/board".into(),
                model: None,
                context_scope: None,
                allow_write_proposals: true,
            })
            .await
            .unwrap();
        assert!(board.assistant_output.unwrap().contains("generated_at"));
    }

    #[tokio::test]
    async fn slash_tasks_models_and_use_model_work() {
        let (service, project) = service_with_project();
        service
            .database
            .update_local_ai_settings(LocalAiSettingsPatch {
                base_url: Some("http://127.0.0.1:9".into()),
                ..Default::default()
            })
            .unwrap();
        let task = service
            .database
            .create_task(NewTask {
                project_id: project.id,
                title: "Blocked task".into(),
                description: None,
                status: TaskStatus::Blocked,
                priority: 1,
                due_at: None,
                scheduled_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                tags: Vec::new(),
            })
            .unwrap();
        let blocked = service
            .run_local_ai_slash_command(None, "/tasks blocked".into())
            .await
            .unwrap();
        assert!(blocked.assistant_output.unwrap().contains(&task.title));

        let models = service
            .run_local_ai_slash_command(Some(blocked.session.id.clone()), "/models".into())
            .await
            .unwrap();
        assert!(models.assistant_output.unwrap().contains("Ollama"));

        let switched = service
            .run_local_ai_slash_command(Some(blocked.session.id), "/use llama3.2:3b".into())
            .await
            .unwrap();
        assert_eq!(switched.session.model.as_deref(), Some("llama3.2:3b"));
    }

    #[tokio::test]
    async fn write_slash_command_proposes_and_confirmation_gates_execution() {
        let (service, _) = service_with_project();
        let turn = service
            .run_local_ai_slash_command(None, "/create task Confirm me".into())
            .await
            .unwrap();
        let call = turn.proposed_tool_calls.first().unwrap();
        assert_eq!(call.status, LocalAiToolCallStatus::Proposed);

        let error = service.execute_local_ai_tool_call(&call.id).unwrap_err();
        assert!(matches!(error, crate::db::CoreError::Validation(_)));

        service.confirm_local_ai_tool_call(&call.id).unwrap();
        let executed = service.execute_local_ai_tool_call(&call.id).unwrap();
        assert_eq!(executed.status, LocalAiToolCallStatus::Executed);
        assert!(
            service
                .database
                .list_tasks()
                .unwrap()
                .iter()
                .any(|task| task.title == "Confirm me")
        );
    }

    #[tokio::test]
    async fn canceled_tool_call_does_not_execute() {
        let (service, _) = service_with_project();
        let turn = service
            .run_local_ai_slash_command(None, "/create task Do not create".into())
            .await
            .unwrap();
        let id = turn.proposed_tool_calls[0].id.clone();
        service.cancel_local_ai_tool_call(&id).unwrap();
        assert!(service.execute_local_ai_tool_call(&id).is_err());
        assert!(
            !service
                .database
                .list_tasks()
                .unwrap()
                .iter()
                .any(|task| task.title == "Do not create")
        );
    }

    #[tokio::test]
    async fn context_and_empty_db_chat_are_safe() {
        let service = AppService::new(Database::in_memory().unwrap());
        service
            .database
            .update_local_ai_settings(LocalAiSettingsPatch {
                base_url: Some("http://127.0.0.1:9".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(
            service
                .build_local_ai_chat_context(LocalAiContextScope::Daily)
                .unwrap()
                .contains("P1 is highest priority")
        );

        let turn = service
            .send_local_ai_chat_message(SendLocalAiChatMessageInput {
                session_id: None,
                message: "Plan my day".into(),
                model: None,
                context_scope: Some(LocalAiContextScope::Daily),
                allow_write_proposals: false,
            })
            .await
            .unwrap();
        assert!(
            turn.assistant_output
                .unwrap()
                .contains("Local AI unavailable")
        );
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
