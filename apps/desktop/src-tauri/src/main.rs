mod commands;

use openmgmt_core::{AppService, Database, default_database_path};
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let database =
        Database::open(default_database_path()).expect("failed to open OpenMgmt database");

    tauri::Builder::default()
        .manage(AppService::new(database))
        .manage(commands::SyncRuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_organizations,
            commands::create_organization,
            commands::update_organization,
            commands::archive_organization,
            commands::list_projects,
            commands::create_project,
            commands::get_project,
            commands::update_project,
            commands::archive_project,
            commands::list_tasks,
            commands::create_task,
            commands::get_task,
            commands::update_task,
            commands::cancel_task,
            commands::get_board_state,
            commands::get_schedule_today,
            commands::get_schedule_week,
            commands::get_schedule_for_day,
            commands::get_unscheduled_tasks,
            commands::get_overdue_tasks,
            commands::auto_start_due_scheduled_tasks,
            commands::schedule_task,
            commands::reschedule_task,
            commands::clear_task_schedule,
            commands::list_schedule_conflicts,
            commands::suggest_next_time_block,
            commands::suggest_tasks_for_time_window,
            commands::complete_scheduled_block,
            commands::skip_scheduled_block,
            commands::generate_schedule_ics,
            commands::start_task,
            commands::complete_task,
            commands::start_task_timer,
            commands::pause_task_timer,
            commands::resume_task_timer,
            commands::stop_task_timer,
            commands::complete_task_with_timer,
            commands::list_task_timer_sessions,
            commands::get_active_timer_session,
            commands::block_task,
            commands::unblock_task,
            commands::list_saved_task_views,
            commands::get_saved_task_view,
            commands::create_saved_task_view,
            commands::update_saved_task_view,
            commands::archive_saved_task_view,
            commands::query_tasks,
            commands::get_scoring_settings,
            commands::update_scoring_settings,
            commands::reset_scoring_settings,
            commands::get_local_ai_settings,
            commands::update_local_ai_settings,
            commands::reset_local_ai_settings,
            commands::test_ollama_connection,
            commands::list_ollama_models,
            commands::run_local_ai_prompt,
            commands::plan_day_with_local_ai,
            commands::summarize_project_with_local_ai,
            commands::triage_tasks_with_local_ai,
            commands::suggest_next_task_with_local_ai,
            commands::rewrite_task_description_with_local_ai,
            commands::list_local_ai_chat_sessions,
            commands::create_local_ai_chat_session,
            commands::archive_local_ai_chat_session,
            commands::update_local_ai_chat_session_access_mode,
            commands::list_local_ai_chat_messages,
            commands::send_local_ai_chat_message,
            commands::confirm_local_ai_tool_call,
            commands::cancel_local_ai_tool_call,
            commands::execute_local_ai_tool_call,
            commands::list_local_ai_tools,
            commands::run_local_ai_slash_command,
            commands::export_tasks_json,
            commands::export_tasks_csv,
            commands::export_all_json,
            commands::backup_sqlite_database,
            commands::get_sync_settings,
            commands::update_sync_settings,
            commands::get_sync_status,
            commands::sync_now,
            commands::test_sync_connection,
            commands::clear_sync_error,
            commands::open_tv_board_window,
            commands::close_tv_board_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running OpenMgmt");
}
