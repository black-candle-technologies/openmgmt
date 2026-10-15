use openmgmt_core::{AgentState, AppService, RepoBinding, TaskAgent};
use openmgmt_sync_client::{
    dispatch::{DEFAULT_BASE_URL, DispatchClient},
    oauth::{desktop_token_store, load_access_token},
};
use serde_json::{Value, json};
use tauri::{Manager, State};

fn settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("dispatch-url.txt"))
}
#[tauri::command]
pub fn dispatch_settings(
    app: tauri::AppHandle,
    base_url: Option<String>,
) -> Result<String, String> {
    let path = settings_path(&app)?;
    if let Some(url) = base_url {
        DispatchClient::new(&url, String::new())?;
        std::fs::write(&path, &url).map_err(|e| e.to_string())?;
        return Ok(url);
    }
    Ok(std::fs::read_to_string(path).unwrap_or_else(|_| DEFAULT_BASE_URL.into()))
}
#[tauri::command]
pub fn explain_task_rank(
    service: State<'_, AppService>,
    id: String,
) -> Result<openmgmt_core::scoring::RankExplanation, String> {
    service
        .database()
        .explain_task_rank(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn dispatch_signed_in() -> Result<bool, String> {
    load_access_token(desktop_token_store())
        .map(|t| t.is_some())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn bind_repository(
    service: State<'_, AppService>,
    project_id: String,
    binding: RepoBinding,
) -> Result<(), String> {
    service
        .database()
        .set_repo_binding(&project_id, &binding)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn dispatch_request(
    app: tauri::AppHandle,
    service: State<'_, AppService>,
    action: String,
    task_id: String,
    id: Option<String>,
    body: Value,
) -> Result<Value, String> {
    let token = load_access_token(desktop_token_store())
        .map_err(|e| e.to_string())?
        .ok_or("Sign in on Sync to use Dispatch")?;
    let database = service.database();
    let task = database.get_task(&task_id).map_err(|e| e.to_string())?;
    let client = DispatchClient::new(&dispatch_settings(app, None)?, token)?;
    let mut body = body;
    if action == "dispatch" || action == "generate" {
        let project = database
            .get_project(&task.project_id)
            .map_err(|e| e.to_string())?;
        let repo = project
            .repo_binding
            .ok_or("Bind a repository in Project Detail first")?;
        body["task_id"] = json!(task_id);
        body["repository"] = json!(repo);
        body["title"] = json!(task.title);
        body["description"] = json!(task.description);
    }
    let response = client.request(&action, id.as_deref(), body).await?;
    if action == "dispatch" || action == "get" {
        if let Some(dispatch_id) = response["id"].as_str() {
            let agent = TaskAgent {
                state: AgentState::from_api(
                    response["state"]
                        .as_str()
                        .or(response["status"].as_str())
                        .unwrap_or("dispatched"),
                ),
                dispatch_id: dispatch_id.into(),
                pr_url: response["pr_url"].as_str().map(str::to_owned),
                updated_at: chrono::Utc::now(),
            };
            database
                .set_task_agent(&task_id, &agent)
                .map_err(|e| e.to_string())?;
        } else {
            return Err("Dispatch response is missing its id".into());
        }
    }
    Ok(response)
}
