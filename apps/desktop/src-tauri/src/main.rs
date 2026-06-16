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
            commands::start_task,
            commands::complete_task,
            commands::block_task,
            commands::unblock_task,
            commands::get_sync_settings,
            commands::update_sync_settings,
            commands::get_sync_status,
            commands::sync_now,
            commands::test_sync_connection,
            commands::clear_sync_error,
            commands::open_tv_board_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running OpenMgmt");
}
