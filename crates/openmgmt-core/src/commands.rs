use crate::{
    db::{Database, Result},
    local_ai::{
        ChatOptions, LocalAiTurnIntent, OllamaClient, PlannedStep, SlashCommand,
        build_chat_messages, build_read_messages, classify_turn, compact_task_lines, local_ai_tool,
        local_ai_tool_registry, parse_action_plan, parse_say_command, parse_slash_command,
        plan_action_from_message, plan_covers_request, plan_day_prompt, plan_from_message,
        plan_is_executable, prompt_messages, rewrite_task_prompt, slash_help,
        suggest_next_task_prompt, summarize_project_prompt, triage_tasks_prompt, workflow_error,
        workflow_response, write_planning_prompt, write_tool_manifest_text,
    },
    models::{
        BoardState, CalendarBlock, LocalAiAccessMode, LocalAiChatMessage, LocalAiChatMessageRecord,
        LocalAiChatResponse, LocalAiChatRole, LocalAiChatSession, LocalAiChatTurn,
        LocalAiConnectionResult, LocalAiContextScope, LocalAiModelListResult, LocalAiSettings,
        LocalAiSettingsPatch, LocalAiToolCall, LocalAiToolCallStatus, LocalAiToolDefinition,
        LocalAiWorkflowResponse, NewOrganization, NewProject, NewSavedTaskView, NewTask,
        Organization, OrganizationPatch, Project, ProjectPatch, ProjectStatus, SavedTaskView,
        SavedTaskViewPatch, ScheduleConflict, ScheduleTaskInput, ScheduledBlockCompletion,
        ScoringSettings, ScoringSettingsPatch, Task, TaskPatch, TaskQueryFilter, TaskSort,
        TaskStatus, TaskTimerSession, TaskWithContext, TimeBlockSuggestion,
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
        let options = ChatOptions {
            num_ctx: settings.context_window,
            ..Default::default()
        };
        OllamaClient::new(&settings, 30)?
            .chat(&settings, model, messages, options)
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
    pub fn update_local_ai_chat_session_access_mode(
        &self,
        id: &str,
        access_mode: LocalAiAccessMode,
    ) -> Result<LocalAiChatSession> {
        self.database
            .update_local_ai_chat_session_access_mode(id, access_mode)
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
    /// A Local AI chat turn. The user message is first *classified* — pure chat,
    /// a workspace read, a write request, or something too vague — and only write
    /// requests ever touch the tool layer. Writes build a complete plan up front
    /// and apply it under the session's access mode.
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
        if let Some(access_mode) = input.access_mode {
            session = self
                .database
                .update_local_ai_chat_session_access_mode(&session.id, access_mode)?;
        }
        let user = self.database.append_local_ai_chat_message(
            &session.id,
            LocalAiChatRole::User,
            input.message.clone(),
            session.model.clone(),
            None,
        )?;
        // Slash commands remain a hidden power-user/debug path.
        if let Some(command) = parse_slash_command(&input.message)? {
            self.handle_local_ai_slash_command(&mut session, &user, command)
                .await?;
            return self.local_ai_chat_turn(session, false);
        }

        let mutated = match classify_turn(&input.message) {
            LocalAiTurnIntent::Chat => self.run_chat_turn(&session, &input.message).await?,
            LocalAiTurnIntent::Read => self.run_read_turn(&session, &input.message).await?,
            LocalAiTurnIntent::Clarify => {
                self.append_assistant(
                    &session,
                    "I can make changes for you — tell me what to do and which task, project, or organization it's about."
                        .into(),
                )?;
                false
            }
            LocalAiTurnIntent::Write => {
                self.run_write_turn(&session, &user, &input.message).await?
            }
        };

        self.local_ai_chat_turn(session, mutated)
    }

    /// Pure conversation: no workspace data, no tools. `say`/`repeat` echo
    /// deterministically; everything else gets a plain chat reply.
    async fn run_chat_turn(&self, session: &LocalAiChatSession, message: &str) -> Result<bool> {
        if let Some(echoed) = parse_say_command(message) {
            self.append_assistant(session, echoed)?;
            return Ok(false);
        }
        let history = self.database.list_local_ai_chat_messages(&session.id)?;
        let messages = build_chat_messages(&history, 8);
        match self.run_ollama_chat(session.model.clone(), messages).await {
            Ok(chat) => self.append_assistant(session, non_empty(chat.content, "…"))?,
            Err(error) => {
                self.append_assistant(session, format!("Local AI unavailable: {error}"))?
            }
        }
        Ok(false)
    }

    /// A workspace question: answer conversationally from injected read context.
    /// No writes, no proposals.
    async fn run_read_turn(&self, session: &LocalAiChatSession, _message: &str) -> Result<bool> {
        let context = self.build_local_ai_chat_context(LocalAiContextScope::Daily)?;
        let history = self.database.list_local_ai_chat_messages(&session.id)?;
        let messages = build_read_messages(&context, &history, 8);
        match self.run_ollama_chat(session.model.clone(), messages).await {
            Ok(chat) => self.append_assistant(
                session,
                non_empty(
                    chat.content,
                    "I don't have anything to add from your workspace.",
                ),
            )?,
            Err(error) => {
                self.append_assistant(session, format!("Local AI unavailable: {error}"))?
            }
        }
        Ok(false)
    }

    /// A write request: build a complete plan, then apply the access mode.
    /// Read only explains; Ask first proposes one grouped plan; Full access
    /// executes the whole plan in order and reports back conversationally.
    async fn run_write_turn(
        &self,
        session: &LocalAiChatSession,
        user: &LocalAiChatMessageRecord,
        message: &str,
    ) -> Result<bool> {
        // A new plan supersedes any earlier un-acted one in this session.
        self.database
            .cancel_pending_local_ai_tool_calls(&session.id)?;

        // Planning may call Ollama (non create-chain writes); a dead server
        // degrades to a friendly message instead of a hard command error.
        let steps = match self.build_write_plan(session, message).await {
            Ok(steps) => steps,
            Err(error) => {
                self.append_assistant(session, format!("Local AI unavailable: {error}"))?;
                return Ok(false);
            }
        };
        if steps.is_empty() {
            self.append_assistant(
                session,
                "I couldn't turn that into a concrete change. Tell me the action and the task, project, or organization to apply it to."
                    .into(),
            )?;
            return Ok(false);
        }

        match session.access_mode {
            LocalAiAccessMode::ReadOnly => {
                self.append_assistant(
                    session,
                    "I can do that, but this chat is in Read Only. Switch to Ask First or Full Access to let me make changes."
                        .into(),
                )?;
                Ok(false)
            }
            LocalAiAccessMode::AskBeforeWrite => {
                // One grouped plan: a Proposed record per step, all tied to this
                // user message so the UI renders a single plan card.
                for step in &steps {
                    self.database.create_local_ai_tool_call(
                        &session.id,
                        Some(user.id.clone()),
                        step.tool_name.clone(),
                        step.arguments.clone(),
                        LocalAiToolCallStatus::Proposed,
                    )?;
                }
                self.append_assistant(session, propose_intro(&steps))?;
                Ok(false)
            }
            LocalAiAccessMode::FullAccess => {
                self.append_assistant(session, "On it.".into())?;
                let outcome = self.execute_plan_steps(session, &user.id, &steps)?;
                self.append_assistant(session, finalize_plan(&outcome))?;
                Ok(outcome.mutated)
            }
        }
    }

    /// Build a complete, validated write plan. The common create-chain is built
    /// deterministically (reliable on tiny models); everything else asks the
    /// model for an `action_plan`, validates it, and repairs once.
    async fn build_write_plan(
        &self,
        session: &LocalAiChatSession,
        message: &str,
    ) -> Result<Vec<PlannedStep>> {
        let deterministic = plan_from_message(message);
        if plan_is_executable(&deterministic) {
            return Ok(deterministic);
        }
        // Common single-shot status changes (complete/start/cancel/…) are also
        // deterministic, so they don't depend on the model emitting valid JSON.
        let action = plan_action_from_message(message);
        if plan_is_executable(&action) {
            return Ok(action);
        }
        let mut steps = self.request_action_plan(session, message).await?;
        // One repair pass if the plan is unusable or misses a named entity.
        if (!plan_is_executable(&steps) || !plan_covers_request(message, &steps).is_empty())
            && let Ok(repaired) = self.request_action_plan(session, message).await
            && plan_is_executable(&repaired)
        {
            steps = repaired;
        }
        // Last resort: fall back to whatever the deterministic extractor found.
        if !plan_is_executable(&steps) {
            steps = deterministic;
        }
        steps.retain(|step| local_ai_tool(&step.tool_name).is_some_and(|tool| tool.write));
        Ok(steps)
    }

    /// One planning round-trip: ask the model for an `action_plan` and parse it.
    async fn request_action_plan(
        &self,
        session: &LocalAiChatSession,
        message: &str,
    ) -> Result<Vec<PlannedStep>> {
        let messages = vec![
            LocalAiChatMessage {
                role: "system".into(),
                content: write_planning_prompt(&write_tool_manifest_text()),
            },
            LocalAiChatMessage {
                role: "user".into(),
                content: format!("Request: {message}\nReturn the action_plan JSON."),
            },
        ];
        let chat = self
            .run_ollama_chat(session.model.clone(), messages)
            .await?;
        Ok(parse_action_plan(&chat.content)
            .map(|plan| plan.steps)
            .unwrap_or_default())
    }

    /// Execute plan steps in order, stopping on the first failure. Each step is
    /// logged and produces a compact tool-result message in the transcript.
    fn execute_plan_steps(
        &self,
        session: &LocalAiChatSession,
        user_message_id: &str,
        steps: &[PlannedStep],
    ) -> Result<PlanOutcome> {
        let mut outcome = PlanOutcome::default();
        for step in steps {
            let Some(tool) = local_ai_tool(&step.tool_name) else {
                continue;
            };
            let mut record = self.database.create_local_ai_tool_call(
                &session.id,
                Some(user_message_id.to_owned()),
                tool.name.clone(),
                step.arguments.clone(),
                LocalAiToolCallStatus::Confirmed,
            )?;
            match self.run_tool(&tool.name, &step.arguments) {
                Ok(result) => {
                    record.result_json = Some(result.clone());
                    record.status = LocalAiToolCallStatus::Executed;
                    record.updated_at = Utc::now();
                    self.database.save_local_ai_tool_call(&record)?;
                    let summary = summarize_tool_result(&tool.name, &result);
                    self.database.append_local_ai_chat_message(
                        &session.id,
                        LocalAiChatRole::Tool,
                        summary.clone(),
                        None,
                        Some(serde_json::json!({
                            "tool_call_id": record.id,
                            "tool_name": tool.name,
                            "result": result,
                        })),
                    )?;
                    outcome.executed_any = true;
                    outcome.mutated |= tool.write;
                    outcome.summaries.push(summary);
                }
                Err(error) => {
                    record.status = LocalAiToolCallStatus::Failed;
                    record.error_message = Some(error.to_string());
                    record.updated_at = Utc::now();
                    self.database.save_local_ai_tool_call(&record)?;
                    self.append_assistant(
                        session,
                        format!("I couldn't {}: {error}", humanize_tool_name(&tool.name)),
                    )?;
                    outcome.failed_any = true;
                    break; // Later steps may depend on this one — stop here.
                }
            }
        }
        Ok(outcome)
    }

    /// Confirm and run a whole proposed plan (Ask First mode). Steps execute in
    /// order, stop on failure, and the turn ends with a conversational summary.
    pub fn confirm_local_ai_plan(&self, session_id: &str) -> Result<LocalAiChatTurn> {
        let session = self.database.get_local_ai_chat_session(session_id)?;
        let steps = self
            .database
            .list_local_ai_tool_calls(session_id)?
            .into_iter()
            .filter(|call| call.status == LocalAiToolCallStatus::Proposed)
            .map(|call| PlannedStep {
                tool_name: call.tool_name,
                arguments: call.arguments_json,
            })
            .collect::<Vec<_>>();
        if steps.is_empty() {
            return self.local_ai_chat_turn(session, false);
        }
        // The proposals were created against the user message; reuse the most
        // recent one so the executed records stay grouped with it.
        let user_message_id = self
            .database
            .list_local_ai_chat_messages(session_id)?
            .into_iter()
            .rev()
            .find(|message| message.role == LocalAiChatRole::User)
            .map(|message| message.id);
        // Mark the proposals consumed, then run a fresh ordered execution.
        self.database
            .cancel_pending_local_ai_tool_calls(session_id)?;
        let outcome =
            self.execute_plan_steps(&session, user_message_id.as_deref().unwrap_or(""), &steps)?;
        self.append_assistant(&session, finalize_plan(&outcome))?;
        self.local_ai_chat_turn(session, outcome.mutated)
    }

    /// Cancel a whole proposed plan (Ask First mode) without running anything.
    pub fn cancel_local_ai_plan(&self, session_id: &str) -> Result<LocalAiChatTurn> {
        let session = self.database.get_local_ai_chat_session(session_id)?;
        let canceled = self
            .database
            .cancel_pending_local_ai_tool_calls(session_id)?;
        if canceled > 0 {
            self.append_assistant(&session, "Canceled that plan.".into())?;
        }
        self.local_ai_chat_turn(session, false)
    }

    fn append_assistant(&self, session: &LocalAiChatSession, content: String) -> Result<()> {
        self.database.append_local_ai_chat_message(
            &session.id,
            LocalAiChatRole::Assistant,
            content,
            session.model.clone(),
            None,
        )?;
        Ok(())
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
            access_mode: None,
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
        match self.run_tool(&call.tool_name, &call.arguments_json) {
            Ok(result) => {
                call.result_json = Some(result.clone());
                call.status = LocalAiToolCallStatus::Executed;
                call.error_message = None;
                call.updated_at = Utc::now();
                self.database.save_local_ai_tool_call(&call)?;
                let _ = self.database.append_local_ai_chat_message(
                    &call.session_id,
                    LocalAiChatRole::Tool,
                    summarize_tool_result(&call.tool_name, &result),
                    None,
                    Some(serde_json::json!({
                        "tool_call_id": call.id,
                        "tool_name": call.tool_name,
                        "result": result,
                    })),
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

    fn local_ai_chat_turn(
        &self,
        session: LocalAiChatSession,
        mutated: bool,
    ) -> Result<LocalAiChatTurn> {
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
            mutated,
        })
    }

    /// Dispatch one validated, resolved tool against the typed service layer.
    /// Natural-language `*_selector` args are resolved to ids here (at execution
    /// time), so proposals that depend on each other resolve once their
    /// prerequisites exist. There is no shell/SQL/filesystem escape hatch.
    fn run_tool(&self, tool_name: &str, args: &serde_json::Value) -> Result<serde_json::Value> {
        use serde_json::{json, to_value};
        let result = match tool_name {
            // --- reads ------------------------------------------------------
            "get_workspace_summary" => json!({
                "organizations": self.database.list_organizations()?.len(),
                "projects": self.database.list_projects()?.len(),
                "tasks": self.database.list_tasks()?.len(),
            }),
            "list_organizations" => to_value(self.database.list_organizations()?)?,
            "list_projects" => to_value(self.database.list_projects()?)?,
            "list_tasks" => to_value(self.database.list_tasks()?)?,
            "get_board" => to_value(self.database.board_state()?)?,
            "get_daily_operations" => json!({
                "board": self.database.board_state()?,
                "overdue": self.database.get_overdue_tasks()?,
                "unscheduled": self.database.get_unscheduled_tasks()?,
            }),
            "get_schedule_today" => to_value(self.database.get_schedule_today()?)?,
            "get_schedule_week" => to_value(self.database.get_schedule_week()?)?,
            "get_unscheduled_tasks" => to_value(self.database.get_unscheduled_tasks()?)?,
            "get_overdue_tasks" => to_value(self.database.get_overdue_tasks()?)?,
            "get_task" => to_value(self.database.get_task(&self.resolve_task_id(args)?)?)?,
            "get_project" => to_value(self.database.get_project(&self.resolve_project_id(args)?)?)?,
            "get_scoring_settings" => to_value(self.database.get_scoring_settings()?)?,
            "get_sync_status" => to_value(self.database.get_sync_status()?)?,
            // --- writes -----------------------------------------------------
            "create_organization" => {
                to_value(self.database.create_organization(NewOrganization {
                    name: required_arg(args, "name")?.into(),
                    slug: None,
                    description: optional_arg(args, "description").map(str::to_owned),
                    color: None,
                    icon: None,
                })?)?
            }
            "update_organization" => {
                let id = self.resolve_org_id(args)?;
                to_value(self.database.update_organization(
                    &id,
                    OrganizationPatch {
                        name: optional_arg(args, "name").map(str::to_owned),
                        description: optional_opt_str(args, "description"),
                        ..Default::default()
                    },
                )?)?
            }
            "archive_organization" => {
                let id = self.resolve_org_id(args)?;
                self.database.archive_organization(&id)?;
                json!({ "archived_organization_id": id })
            }
            "create_project" => to_value(self.database.create_project(NewProject {
                organization_id: self.resolve_org_id(args)?,
                name: required_arg(args, "name")?.into(),
                slug: None,
                description: optional_arg(args, "description").map(str::to_owned),
                project_type: Default::default(),
                status: Default::default(),
                priority: optional_i32_arg(args, "priority").unwrap_or(3),
                deadline: None,
                repo_url: None,
                notes: None,
            })?)?,
            "update_project" => {
                let id = self.resolve_project_id(args)?;
                to_value(self.database.update_project(
                    &id,
                    ProjectPatch {
                        name: optional_arg(args, "name").map(str::to_owned),
                        description: optional_opt_str(args, "description"),
                        priority: optional_i32_arg(args, "priority"),
                        status: optional_enum_arg::<ProjectStatus>(args, "status"),
                        ..Default::default()
                    },
                )?)?
            }
            "archive_project" => {
                let id = self.resolve_project_id(args)?;
                self.database.archive_project(&id)?;
                json!({ "archived_project_id": id })
            }
            "create_task" => to_value(self.database.create_task(NewTask {
                project_id: self.resolve_project_id(args)?,
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
            })?)?,
            "update_task" => {
                let id = self.resolve_task_id(args)?;
                to_value(self.database.update_task(
                    &id,
                    TaskPatch {
                        title: optional_arg(args, "title").map(str::to_owned),
                        description: optional_opt_str(args, "description"),
                        status: optional_enum_arg::<TaskStatus>(args, "status"),
                        priority: optional_i32_arg(args, "priority"),
                        ..Default::default()
                    },
                )?)?
            }
            "start_task" => to_value(self.database.transition_task(
                &self.resolve_task_id(args)?,
                TaskStatus::InProgress,
                None,
            )?)?,
            "block_task" => to_value(
                self.database.transition_task(
                    &self.resolve_task_id(args)?,
                    TaskStatus::Blocked,
                    Some(
                        optional_arg(args, "reason")
                            .unwrap_or("Blocked by Local AI")
                            .into(),
                    ),
                )?,
            )?,
            "unblock_task" => to_value(self.database.transition_task(
                &self.resolve_task_id(args)?,
                TaskStatus::Ready,
                None,
            )?)?,
            "complete_task" => to_value(self.database.transition_task(
                &self.resolve_task_id(args)?,
                TaskStatus::Done,
                None,
            )?)?,
            "cancel_task" => to_value(self.database.transition_task(
                &self.resolve_task_id(args)?,
                TaskStatus::Canceled,
                None,
            )?)?,
            "schedule_task" => to_value(self.database.schedule_task(
                &self.resolve_task_id(args)?,
                schedule_input_from_args(args)?,
            )?)?,
            "reschedule_task" => to_value(self.database.reschedule_task(
                &self.resolve_task_id(args)?,
                schedule_input_from_args(args)?,
            )?)?,
            "clear_task_schedule" => to_value(
                self.database
                    .clear_task_schedule(&self.resolve_task_id(args)?)?,
            )?,
            "start_task_timer" => to_value(
                self.database
                    .start_task_timer(&self.resolve_task_id(args)?)?,
            )?,
            "pause_task_timer" => to_value(
                self.database
                    .pause_task_timer(&self.resolve_task_id(args)?)?,
            )?,
            "resume_task_timer" => to_value(
                self.database
                    .resume_task_timer(&self.resolve_task_id(args)?)?,
            )?,
            "stop_task_timer" => to_value(
                self.database
                    .stop_task_timer(&self.resolve_task_id(args)?)?,
            )?,
            "complete_task_with_timer" => to_value(
                self.database
                    .complete_task_with_timer(&self.resolve_task_id(args)?)?,
            )?,
            "update_scoring_settings" => to_value(self.database.update_scoring_settings(
                ScoringSettingsPatch {
                    priority_weight: optional_i32_arg(args, "priority_weight"),
                    overdue_boost: optional_i32_arg(args, "overdue_boost"),
                    due_soon_boost: optional_i32_arg(args, "due_soon_boost"),
                    in_progress_boost: optional_i32_arg(args, "in_progress_boost"),
                    pinned_boost: optional_i32_arg(args, "pinned_boost"),
                    blocked_penalty: optional_i32_arg(args, "blocked_penalty"),
                    waiting_penalty: optional_i32_arg(args, "waiting_penalty"),
                    paused_project_penalty: optional_i32_arg(args, "paused_project_penalty"),
                    due_soon_window_hours: optional_i32_arg(args, "due_soon_window_hours"),
                },
            )?)?,
            "reset_scoring_settings" => to_value(self.database.reset_scoring_settings()?)?,
            _ => return Err(crate::db::CoreError::NotFound("local AI tool")),
        };
        Ok(result)
    }

    /// Resolve an organization id from `organization_id`, `organization_selector`
    /// / `organization` (a name, or "any" → first active org), defaulting to the
    /// first active org when nothing is given.
    fn resolve_org_id(&self, args: &serde_json::Value) -> Result<String> {
        if let Some(id) = selector_arg(args, "organization_id") {
            return Ok(id.to_owned());
        }
        let selector = selector_arg(args, "organization_selector")
            .or_else(|| selector_arg(args, "organization"))
            .unwrap_or("any");
        let orgs = self.database.list_organizations()?;
        if orgs.is_empty() {
            return Err(crate::db::CoreError::Validation(
                "there are no organizations yet — create one first".into(),
            ));
        }
        if is_any_selector(selector) {
            return Ok(orgs[0].id.clone());
        }
        resolve_by_name(
            orgs.iter().map(|org| (org.id.clone(), org.name.clone())),
            selector,
            "organization",
        )
    }

    /// Resolve a project id from `project_id`, `project_selector` / `project` (a
    /// name, or "any" → first active project), defaulting to the first project.
    fn resolve_project_id(&self, args: &serde_json::Value) -> Result<String> {
        if let Some(id) = selector_arg(args, "project_id") {
            return Ok(id.to_owned());
        }
        let selector = selector_arg(args, "project_selector")
            .or_else(|| selector_arg(args, "project"))
            .unwrap_or("any");
        let projects = self.database.list_projects()?;
        if projects.is_empty() {
            return Err(crate::db::CoreError::Validation(
                "there are no projects yet — create one first".into(),
            ));
        }
        if is_any_selector(selector) {
            return Ok(projects[0].id.clone());
        }
        resolve_by_name(
            projects
                .iter()
                .map(|project| (project.id.clone(), project.name.clone())),
            selector,
            "project",
        )
    }

    /// Resolve a task id from `task_id` or `task_selector` / `task` (matched by
    /// title). Unlike orgs/projects there is no "any" default — acting on a task
    /// requires naming one.
    fn resolve_task_id(&self, args: &serde_json::Value) -> Result<String> {
        if let Some(id) = selector_arg(args, "task_id") {
            return Ok(id.to_owned());
        }
        let Some(selector) =
            selector_arg(args, "task_selector").or_else(|| selector_arg(args, "task"))
        else {
            return Err(crate::db::CoreError::Validation(
                "which task? give a task title or id".into(),
            ));
        };
        resolve_by_name(
            self.database
                .list_tasks()?
                .iter()
                .map(|task| (task.id.clone(), task.title.clone())),
            selector,
            "task",
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

/// For `Option<Option<String>>` patch fields: `Some(Some(text))` when the key is
/// present with a string, `None` (leave unchanged) when absent.
fn optional_opt_str(args: &serde_json::Value, key: &str) -> Option<Option<String>> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(|value| Some(value.to_owned()))
}

fn optional_enum_arg<T: std::str::FromStr>(args: &serde_json::Value, key: &str) -> Option<T> {
    optional_arg(args, key).and_then(|value| value.parse::<T>().ok())
}

/// A non-empty, trimmed string argument (used for id/selector lookups).
fn selector_arg<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_any_selector(selector: &str) -> bool {
    matches!(
        selector.trim().to_lowercase().as_str(),
        "any" | "*" | "first" | "any organization" | "any org" | "any project"
    )
}

/// Case-insensitive name resolution: exact matches win, else substring matches.
/// One match → its id; several → ask which; none → say so.
fn resolve_by_name<I>(candidates: I, selector: &str, kind: &str) -> Result<String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let needle = selector.trim().to_lowercase();
    let all = candidates.into_iter().collect::<Vec<_>>();
    let exact = all
        .iter()
        .filter(|(_, name)| name.to_lowercase() == needle)
        .collect::<Vec<_>>();
    let chosen = if exact.is_empty() {
        all.iter()
            .filter(|(_, name)| name.to_lowercase().contains(&needle))
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match chosen.as_slice() {
        [(id, _)] => Ok(id.clone()),
        [] => Err(crate::db::CoreError::Validation(format!(
            "no {kind} matches \"{selector}\""
        ))),
        many => {
            let names = many
                .iter()
                .map(|(_, name)| name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            Err(crate::db::CoreError::Validation(format!(
                "multiple {kind}s match \"{selector}\": {names}. Which one?"
            )))
        }
    }
}

fn humanize_tool_name(name: &str) -> String {
    let mut text = name.replace('_', " ");
    if let Some(first) = text.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    text
}

/// Turn a tool result into one human-readable line for the transcript and the
/// model's follow-up context. Writes get a friendly verb; reads/timers get
/// compact JSON, truncated to keep the prompt small.
fn summarize_tool_result(tool_name: &str, result: &serde_json::Value) -> String {
    let label = result
        .get("name")
        .and_then(|value| value.as_str())
        .or_else(|| result.get("title").and_then(|value| value.as_str()));
    let labeled = |verb: &str, kind: &str| match label {
        Some(name) => format!("{verb} {kind} \"{name}\"."),
        None => format!("{verb} {kind}."),
    };
    match tool_name {
        "create_organization" => labeled("Created", "organization"),
        "create_project" => labeled("Created", "project"),
        "create_task" => labeled("Created", "task"),
        "update_organization" => labeled("Updated", "organization"),
        "update_project" => labeled("Updated", "project"),
        "update_task" => labeled("Updated", "task"),
        "start_task" => labeled("Started", "task"),
        "complete_task" | "complete_task_with_timer" => labeled("Completed", "task"),
        "block_task" => labeled("Blocked", "task"),
        "unblock_task" => labeled("Unblocked", "task"),
        "cancel_task" => labeled("Canceled", "task"),
        "archive_organization" => "Archived the organization.".into(),
        "archive_project" => "Archived the project.".into(),
        "schedule_task" => "Scheduled the task.".into(),
        "reschedule_task" => "Rescheduled the task.".into(),
        "clear_task_schedule" => labeled("Cleared schedule for", "task"),
        _ => {
            let compact = result.to_string().chars().take(600).collect::<String>();
            format!("{tool_name}: {compact}")
        }
    }
}

/// Result of executing a write plan in order.
#[derive(Debug, Default, Clone)]
pub struct PlanOutcome {
    pub executed_any: bool,
    pub failed_any: bool,
    pub mutated: bool,
    pub summaries: Vec<String>,
}

/// A short intro shown above an Ask-First plan card.
fn propose_intro(steps: &[PlannedStep]) -> String {
    let count = steps.len();
    if count == 1 {
        "Here's what I'll do — confirm to apply it.".into()
    } else {
        format!("Here's my {count}-step plan — confirm to apply it.")
    }
}

/// The conversational wrap-up after a plan runs: report what happened, or that
/// nothing did. Falls back gracefully when the model isn't involved.
fn finalize_plan(outcome: &PlanOutcome) -> String {
    if outcome.summaries.is_empty() {
        return if outcome.failed_any {
            "I couldn't complete that — see the error above.".into()
        } else {
            "Nothing to do.".into()
        };
    }
    let body = outcome.summaries.join(" ");
    if outcome.failed_any {
        format!("{body} I stopped there after an error.")
    } else {
        format!("Done. {body}")
    }
}

/// Trim a model reply, substituting a fallback when it's blank.
fn non_empty(content: String, fallback: &str) -> String {
    if content.trim().is_empty() {
        fallback.into()
    } else {
        content
    }
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
                access_mode: None,
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
                access_mode: None,
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
                access_mode: None,
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

    fn service_with_org() -> AppService {
        let database = Database::in_memory().unwrap();
        database
            .create_organization(NewOrganization {
                name: "Black Candle".into(),
                slug: None,
                description: None,
                color: None,
                icon: None,
            })
            .unwrap();
        AppService::new(database)
    }

    /// The canonical multi-step request. `build_write_plan` resolves this
    /// deterministically, so these turn tests never touch Ollama.
    const CREATE_CHAIN: &str = "create an organization called localtest, and then a project under that called localproject and then a task under that called localtask";

    async fn send(
        service: &AppService,
        session_id: Option<String>,
        mode: LocalAiAccessMode,
        message: &str,
    ) -> LocalAiChatTurn {
        service
            .send_local_ai_chat_message(SendLocalAiChatMessageInput {
                session_id,
                message: message.into(),
                model: None,
                context_scope: None,
                access_mode: Some(mode),
                allow_write_proposals: true,
            })
            .await
            .unwrap()
    }

    #[test]
    fn new_session_defaults_to_ask_before_write() {
        let service = AppService::new(Database::in_memory().unwrap());
        let session = service.create_local_ai_chat_session(None, None).unwrap();
        assert_eq!(session.access_mode, LocalAiAccessMode::AskBeforeWrite);
    }

    #[test]
    fn switching_access_mode_persists() {
        let service = AppService::new(Database::in_memory().unwrap());
        let session = service.create_local_ai_chat_session(None, None).unwrap();
        service
            .update_local_ai_chat_session_access_mode(&session.id, LocalAiAccessMode::FullAccess)
            .unwrap();
        let reloaded = service
            .database
            .get_local_ai_chat_session(&session.id)
            .unwrap();
        assert_eq!(reloaded.access_mode, LocalAiAccessMode::FullAccess);
    }

    #[tokio::test]
    async fn full_access_executes_whole_plan_in_order() {
        let service = AppService::new(Database::in_memory().unwrap());
        let turn = send(&service, None, LocalAiAccessMode::FullAccess, CREATE_CHAIN).await;
        assert!(turn.mutated);
        assert!(turn.proposed_tool_calls.is_empty());

        // All three created, in dependency order (task under project under org).
        let org = service.database.list_organizations().unwrap();
        let org = org.iter().find(|o| o.name == "localtest").unwrap();
        let project = service.database.list_projects().unwrap();
        let project = project.iter().find(|p| p.name == "localproject").unwrap();
        assert_eq!(project.organization_id, org.id);
        let task = service.database.list_tasks().unwrap();
        let task = task.iter().find(|t| t.title == "localtask").unwrap();
        assert_eq!(task.project_id, project.id);
        // One conversational wrap-up, no follow-up needed.
        assert!(turn.assistant_output.unwrap().starts_with("Done"));
    }

    #[tokio::test]
    async fn ask_first_proposes_one_grouped_plan_then_confirms() {
        let service = AppService::new(Database::in_memory().unwrap());
        let turn = send(
            &service,
            None,
            LocalAiAccessMode::AskBeforeWrite,
            CREATE_CHAIN,
        )
        .await;

        // One grouped plan of three proposals; nothing created yet.
        assert_eq!(turn.proposed_tool_calls.len(), 3);
        assert!(!turn.mutated);
        assert!(service.database.list_organizations().unwrap().is_empty());

        // Confirm the whole plan; all three are created in order.
        let confirmed = service.confirm_local_ai_plan(&turn.session.id).unwrap();
        assert!(confirmed.mutated);
        assert!(confirmed.proposed_tool_calls.is_empty());
        let project = service
            .database
            .list_projects()
            .unwrap()
            .into_iter()
            .find(|p| p.name == "localproject")
            .unwrap();
        let task = service
            .database
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|t| t.title == "localtask")
            .unwrap();
        assert_eq!(task.project_id, project.id);
    }

    #[tokio::test]
    async fn read_only_blocks_whole_write_plan() {
        let service = AppService::new(Database::in_memory().unwrap());
        let turn = send(&service, None, LocalAiAccessMode::ReadOnly, CREATE_CHAIN).await;

        assert!(!turn.mutated);
        assert!(turn.proposed_tool_calls.is_empty());
        assert!(service.database.list_organizations().unwrap().is_empty());
        assert!(service.database.list_projects().unwrap().is_empty());
        assert!(turn.assistant_output.unwrap().contains("Read Only"));
    }

    #[tokio::test]
    async fn pure_chat_echoes_without_touching_tools() {
        let service = AppService::new(Database::in_memory().unwrap());
        // Even in Full Access, "say X" is conversational — no tools, no writes.
        let turn = send(
            &service,
            None,
            LocalAiAccessMode::FullAccess,
            "say the words something",
        )
        .await;
        assert_eq!(turn.assistant_output.as_deref(), Some("something"));
        assert!(!turn.mutated);
        assert!(
            service
                .database
                .list_local_ai_tool_calls(&turn.session.id)
                .unwrap()
                .is_empty()
        );

        // "say clear schedule" must not become a clear_task_schedule call.
        let turn = send(
            &service,
            Some(turn.session.id.clone()),
            LocalAiAccessMode::FullAccess,
            "say the words \"clear schedule\"",
        )
        .await;
        assert_eq!(turn.assistant_output.as_deref(), Some("clear schedule"));
        assert!(
            service
                .database
                .list_local_ai_tool_calls(&turn.session.id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn resolver_handles_id_name_case_any_and_clarifications() {
        let service = service_with_org();
        let org = service.database.list_organizations().unwrap()[0].clone();

        // By id, exact name, case-insensitive name, and "any".
        assert_eq!(
            service
                .resolve_org_id(&serde_json::json!({ "organization_id": org.id }))
                .unwrap(),
            org.id
        );
        assert_eq!(
            service
                .resolve_org_id(&serde_json::json!({ "organization_selector": "Black Candle" }))
                .unwrap(),
            org.id
        );
        assert_eq!(
            service
                .resolve_org_id(&serde_json::json!({ "organization_selector": "black candle" }))
                .unwrap(),
            org.id
        );
        assert_eq!(
            service
                .resolve_org_id(&serde_json::json!({ "organization_selector": "any" }))
                .unwrap(),
            org.id
        );

        // Two projects sharing a prefix → ambiguous; unknown name → no match.
        service
            .database
            .create_project(NewProject {
                organization_id: org.id.clone(),
                name: "Alpha One".into(),
                slug: None,
                description: None,
                project_type: Default::default(),
                status: Default::default(),
                priority: 3,
                deadline: None,
                repo_url: None,
                notes: None,
            })
            .unwrap();
        service
            .database
            .create_project(NewProject {
                organization_id: org.id.clone(),
                name: "Alpha Two".into(),
                slug: None,
                description: None,
                project_type: Default::default(),
                status: Default::default(),
                priority: 3,
                deadline: None,
                repo_url: None,
                notes: None,
            })
            .unwrap();
        assert!(
            service
                .resolve_project_id(&serde_json::json!({ "project_selector": "Alpha" }))
                .is_err()
        );
        assert!(
            service
                .resolve_project_id(&serde_json::json!({ "project_selector": "Nope" }))
                .is_err()
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

    /// End-to-end against a real Ollama (needs qwen3:1.7b). Run with
    /// `cargo test -p openmgmt-core live_new_flow_smoke -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_new_flow_smoke() {
        let service = AppService::new(Database::in_memory().unwrap());

        // Capability enrichment from /api/show.
        let models = service.list_ollama_models().await.unwrap().models;
        let qwen = models.iter().find(|m| m.name == "qwen3:1.7b").unwrap();
        assert!(qwen.supports_tools && qwen.supports_thinking && !qwen.is_embedding_model);
        assert!(qwen.context_length.is_some());

        // Pure chat echoes deterministically — no model JSON, no tools.
        let chat = send(
            &service,
            None,
            LocalAiAccessMode::FullAccess,
            "say the words something",
        )
        .await;
        assert_eq!(chat.assistant_output.as_deref(), Some("something"));

        // A create-chain, then a status change — each completes in one turn.
        let setup = send(
            &service,
            None,
            LocalAiAccessMode::FullAccess,
            "create an org called Acme, a project called Web, and a task called Draft",
        )
        .await;
        assert!(setup.mutated);
        let complete = send(
            &service,
            Some(setup.session.id.clone()),
            LocalAiAccessMode::FullAccess,
            "complete the task called Draft",
        )
        .await;
        assert!(complete.mutated);
        let task = service
            .database
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|t| t.title == "Draft")
            .unwrap();
        assert_eq!(task.status, TaskStatus::Done);
    }
}
