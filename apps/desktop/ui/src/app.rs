use crate::{
    api,
    sync::{SyncOnceResult, server_url_hint, status_label, sync_result_summary},
};
use chrono::{DateTime, Utc};
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use openmgmt_core::{
    BoardState, NewOrganization, NewProject, NewTask, Organization, OrganizationPatch, Project,
    ProjectPatch, ProjectStatus, ProjectType, ScoredTask, SyncConnectionState, SyncSettings,
    SyncSettingsPatch, SyncStatus, Task, TaskPatch, TaskStatus,
};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    fn tauri_invoke(command: &str, args: JsValue) -> js_sys::Promise;
}

#[derive(Clone, PartialEq)]
enum Page {
    Dashboard,
    Organizations,
    Projects,
    Project(String),
    Tasks,
    Today,
    Sync,
}

#[derive(Clone, Default)]
struct Snapshot {
    organizations: Vec<Organization>,
    projects: Vec<Project>,
    tasks: Vec<Task>,
    board: BoardState,
}

#[derive(Clone, Copy)]
struct AppState {
    snapshot: RwSignal<Snapshot>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
}

impl AppState {
    fn refresh(self) {
        spawn_local(async move {
            self.reload().await;
        });
    }

    async fn reload(self) {
        self.loading.set(true);
        match load_snapshot().await {
            Ok(snapshot) => {
                self.snapshot.set(snapshot);
                self.error.set(None);
            }
            Err(error) => self.fail("Refresh failed", error),
        }
        self.loading.set(false);
    }

    fn fail(self, context: &str, error: String) {
        let message = format!("{context}: {error}");
        web_sys::console::error_1(&JsValue::from_str(&message));
        self.error.set(Some(message));
    }
}

async fn invoke<T: DeserializeOwned>(command: &str, args: impl Serialize) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|error| error.to_string())?;
    let value = JsFuture::from(tauri_invoke(command, args))
        .await
        .map_err(js_error_message)?;
    serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
}

async fn load_snapshot() -> Result<Snapshot, String> {
    Ok(Snapshot {
        organizations: invoke("list_organizations", serde_json::json!({})).await?,
        projects: invoke("list_projects", serde_json::json!({})).await?,
        tasks: invoke("list_tasks", serde_json::json!({})).await?,
        board: invoke("get_board_state", serde_json::json!({})).await?,
    })
}

async fn finish_action<T>(
    state: AppState,
    result: Result<T, String>,
    success: &'static str,
    context: &'static str,
) {
    match result {
        Ok(_) => {
            state.notice.set(Some(success.into()));
            state.reload().await;
        }
        Err(error) => state.fail(context, error),
    }
}

fn js_error_message(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| {
        js_sys::JSON::stringify(&value)
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "Unknown Tauri invoke error".into())
    })
}

fn is_board_window() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let query_mode = window
        .location()
        .search()
        .ok()
        .is_some_and(|query| query.contains("board=1"));
    let initialized_mode =
        js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("__OPENMGMT_BOARD__"))
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    query_mode || initialized_mode
}

fn input_value(node: NodeRef<leptos::html::Input>) -> String {
    node.get().map(|input| input.value()).unwrap_or_default()
}

fn textarea_value(node: NodeRef<leptos::html::Textarea>) -> String {
    node.get().map(|input| input.value()).unwrap_or_default()
}

fn select_value(node: NodeRef<leptos::html::Select>) -> String {
    node.get().map(|input| input.value()).unwrap_or_default()
}

fn optional_text(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_i32(value: String) -> Option<i32> {
    value.trim().parse().ok()
}

fn parse_datetime_local(value: String) -> Result<Option<DateTime<Utc>>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let date = js_sys::Date::new(&JsValue::from_str(&value));
    let milliseconds = date.get_time();
    if milliseconds.is_nan() {
        return Err(format!("Invalid date and time: {value}"));
    }
    DateTime::from_timestamp_millis(milliseconds as i64)
        .map(Some)
        .ok_or_else(|| format!("Date is outside the supported range: {value}"))
}

fn datetime_local_value(value: Option<DateTime<Utc>>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let date = js_sys::Date::new(&JsValue::from_f64(value.timestamp_millis() as f64));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

fn confirmed(message: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.confirm_with_message(message).ok())
        .unwrap_or(false)
}

fn project_type_options() -> [(ProjectType, &'static str); 9] {
    [
        (ProjectType::Software, "Software"),
        (ProjectType::Writing, "Writing"),
        (ProjectType::Business, "Business"),
        (ProjectType::FilmStory, "Film / story"),
        (ProjectType::MarketingPr, "Marketing / PR"),
        (ProjectType::Research, "Research"),
        (ProjectType::Operations, "Operations"),
        (ProjectType::Personal, "Personal"),
        (ProjectType::Other, "Other"),
    ]
}

fn project_status_options() -> [(ProjectStatus, &'static str); 3] {
    [
        (ProjectStatus::Active, "Active"),
        (ProjectStatus::Paused, "Paused"),
        (ProjectStatus::Completed, "Completed"),
    ]
}

fn task_status_options() -> [(TaskStatus, &'static str); 8] {
    [
        (TaskStatus::Inbox, "Inbox"),
        (TaskStatus::Backlog, "Backlog"),
        (TaskStatus::Scheduled, "Scheduled"),
        (TaskStatus::Ready, "Ready"),
        (TaskStatus::InProgress, "In progress"),
        (TaskStatus::Blocked, "Blocked"),
        (TaskStatus::Waiting, "Waiting"),
        (TaskStatus::Done, "Done"),
    ]
}

#[component]
pub fn App() -> impl IntoView {
    let board_only = is_board_window();
    let state = AppState {
        snapshot: RwSignal::new(Snapshot::default()),
        error: RwSignal::new(None),
        notice: RwSignal::new(None),
        loading: RwSignal::new(true),
    };
    let now = RwSignal::new(Utc::now());
    state.refresh();

    let board_refresh = Interval::new(10_000, move || state.refresh());
    board_refresh.forget();
    let clock_refresh = Interval::new(1_000, move || now.set(Utc::now()));
    clock_refresh.forget();

    if board_only {
        return view! {
            <BoardView
                board=Signal::derive(move || state.snapshot.get().board)
                error=state.error
                loading=state.loading
                now
                state
            />
        }
        .into_any();
    }

    let page = RwSignal::new(Page::Dashboard);
    view! {
        <div class="shell">
            <aside class="sidebar">
                <div class="brand"><span>OM</span><div><strong>OpenMgmt</strong><small>OPERATIONS DESK</small></div></div>
                <nav>
                    <NavButton label="Dashboard" target=Page::Dashboard page />
                    <NavButton label="Organizations" target=Page::Organizations page />
                    <NavButton label="Projects" target=Page::Projects page />
                    <NavButton label="Tasks" target=Page::Tasks page />
                    <NavButton label="Today" target=Page::Today page />
                    <NavButton label="Sync" target=Page::Sync page />
                </nav>
                <div class="sidebar-actions">
                    <button class="ghost dark" on:click=move |_| state.refresh()>"Refresh data"</button>
                    <button class="primary tv" on:click=move |_| {
                        spawn_local(async move {
                            match invoke::<()>("open_tv_board_window", serde_json::json!({})).await {
                                Ok(_) => state.notice.set(Some("TV board opened.".into())),
                                Err(error) => state.fail("Could not open TV board", error),
                            }
                        });
                    }>"Open TV Board"</button>
                </div>
                <p class="local"><i></i>" Local database"</p>
            </aside>
            <main class="content">
                <Feedback state />
                {move || match page.get() {
                    Page::Dashboard => view! { <Dashboard state /> }.into_any(),
                    Page::Organizations => view! { <OrganizationsView state /> }.into_any(),
                    Page::Projects => view! { <ProjectsView state page /> }.into_any(),
                    Page::Project(id) => view! { <ProjectView id state /> }.into_any(),
                    Page::Tasks => view! { <TasksView state /> }.into_any(),
                    Page::Today => view! {
                        <TodayView board=Signal::derive(move || state.snapshot.get().board) state />
                    }.into_any(),
                    Page::Sync => view! { <SyncView /> }.into_any(),
                }}
            </main>
        </div>
    }
    .into_any()
}

#[component]
fn Feedback(state: AppState) -> impl IntoView {
    view! {
        {move || state.error.get().map(|message| view! {
            <div class="feedback error"><span>{message}</span><button on:click=move |_| state.error.set(None)>"Dismiss"</button></div>
        })}
        {move || state.notice.get().map(|message| view! {
            <div class="feedback notice"><span>{message}</span><button on:click=move |_| state.notice.set(None)>"Dismiss"</button></div>
        })}
        {move || state.loading.get().then(|| view! { <div class="loading-bar">"Refreshing local data..."</div> })}
    }
}

#[component]
fn NavButton(label: &'static str, target: Page, page: RwSignal<Page>) -> impl IntoView {
    let active_target = target.clone();
    view! {
        <button class:active=move || page.get() == active_target on:click=move |_| page.set(target.clone())>{label}</button>
    }
}

#[component]
fn Header(eyebrow: &'static str, title: &'static str, description: &'static str) -> impl IntoView {
    view! {
        <header class="page-header"><div><p class="eyebrow">{eyebrow}</p><h1>{title}</h1><p>{description}</p></div></header>
    }
}

#[derive(Clone, Copy)]
struct SyncViewState {
    settings: RwSignal<Option<SyncSettings>>,
    status: RwSignal<Option<SyncStatus>>,
    enabled: RwSignal<bool>,
    server_url: RwSignal<String>,
    device_name: RwSignal<String>,
    loading: RwSignal<bool>,
    action: RwSignal<Option<&'static str>>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    result: RwSignal<Option<SyncOnceResult>>,
}

impl SyncViewState {
    fn load(self) {
        spawn_local(async move {
            self.reload().await;
        });
    }

    async fn reload(self) {
        self.loading.set(true);
        let settings = api::get_sync_settings().await;
        let status = api::get_sync_status().await;
        match settings {
            Ok(settings) => {
                self.enabled.set(settings.enabled);
                self.server_url
                    .set(settings.server_url.clone().unwrap_or_default());
                self.device_name.set(settings.device_name.clone());
                self.settings.set(Some(settings));
            }
            Err(error) => self
                .error
                .set(Some(format!("Could not load sync settings: {error}"))),
        }
        match status {
            Ok(status) => self.status.set(Some(status)),
            Err(error) => self
                .error
                .set(Some(format!("Could not load sync status: {error}"))),
        }
        self.loading.set(false);
    }

    async fn reload_status(self) {
        match api::get_sync_status().await {
            Ok(status) => self.status.set(Some(status)),
            Err(error) => self
                .error
                .set(Some(format!("Could not reload sync status: {error}"))),
        }
    }

    fn start_action(self, action: &'static str) {
        self.action.set(Some(action));
        self.error.set(None);
        self.notice.set(None);
    }

    fn finish_action(self) {
        self.action.set(None);
    }
}

fn format_sync_time(value: Option<DateTime<Utc>>) -> String {
    let Some(value) = value else {
        return "Never".into();
    };
    let date = js_sys::Date::new(&JsValue::from_f64(value.timestamp_millis() as f64));
    date.to_locale_string("en-US", &JsValue::UNDEFINED).into()
}

#[component]
fn SyncView() -> impl IntoView {
    let state = SyncViewState {
        settings: RwSignal::new(None),
        status: RwSignal::new(None),
        enabled: RwSignal::new(false),
        server_url: RwSignal::new(String::new()),
        device_name: RwSignal::new(String::new()),
        loading: RwSignal::new(true),
        action: RwSignal::new(None),
        error: RwSignal::new(None),
        notice: RwSignal::new(None),
        result: RwSignal::new(None),
    };
    state.load();

    view! {
        <Header
            eyebrow="OPTIONAL SYNC"
            title="Sync"
            description="Connect this local workspace to an OpenMgmt server when you choose."
        />
        <section class="panel sync-intro">
            <p>"OpenMgmt is local-first. Sync is optional. You can run an OpenMgmt server locally or in the cloud. Manual sync pushes local changes and pulls remote changes."</p>
        </section>

        {move || state.loading.get().then(|| view! {
            <div class="loading-bar">"Loading sync settings..."</div>
        })}
        {move || state.error.get().map(|message| view! {
            <div class="feedback error">
                <span>{message}</span>
                <button on:click=move |_| state.error.set(None)>"Dismiss"</button>
            </div>
        })}
        {move || state.notice.get().map(|message| view! {
            <div class="feedback notice">
                <span>{message}</span>
                <button on:click=move |_| state.notice.set(None)>"Dismiss"</button>
            </div>
        })}

        <section class="sync-grid">
            <form class="panel sync-form" on:submit=move |event| {
                event.prevent_default();
                state.start_action("save");
                let patch = SyncSettingsPatch {
                    enabled: Some(state.enabled.get()),
                    server_url: Some(optional_text(state.server_url.get())),
                    device_name: Some(state.device_name.get()),
                    ..SyncSettingsPatch::default()
                };
                spawn_local(async move {
                    match api::update_sync_settings(patch).await {
                        Ok(settings) => {
                            state.settings.set(Some(settings));
                            state.notice.set(Some("Sync settings saved.".into()));
                        }
                        Err(error) => state.error.set(Some(format!("Could not save sync settings: {error}"))),
                    }
                    state.reload_status().await;
                    state.finish_action();
                });
            }>
                <div class="section-title">
                    <div><p class="eyebrow">CONFIGURATION</p><h2>"Device settings"</h2></div>
                </div>
                <label class="sync-toggle">
                    <input
                        type="checkbox"
                        prop:checked=move || state.enabled.get()
                        on:change=move |event| state.enabled.set(event_target_checked(&event))
                    />
                    <span><strong>"Enable sync"</strong><small>"Local data remains available when sync is off."</small></span>
                </label>
                <label class="field">
                    <span>"Server URL"</span>
                    <input
                        type="url"
                        placeholder="http://127.0.0.1:8787"
                        prop:value=move || state.server_url.get()
                        on:input=move |event| state.server_url.set(event_target_value(&event))
                    />
                    {move || server_url_hint(&state.server_url.get()).map(|hint| view! {
                        <small class="field-hint warning">{hint}</small>
                    })}
                    {move || (state.enabled.get() && state.server_url.get().trim().is_empty()).then(|| view! {
                        <small class="field-hint">"Sync will be saved as enabled but Not configured."</small>
                    })}
                </label>
                <label class="field">
                    <span>"Device name"</span>
                    <input
                        type="text"
                        placeholder="Local device"
                        prop:value=move || state.device_name.get()
                        on:input=move |event| state.device_name.set(event_target_value(&event))
                    />
                </label>
                <button class="primary" disabled=move || state.action.get().is_some()>
                    {move || if state.action.get() == Some("save") { "Saving..." } else { "Save settings" }}
                </button>
            </form>

            <section class="panel sync-status-panel">
                <div class="section-title">
                    <div><p class="eyebrow">CONNECTION</p><h2>"Sync status"</h2></div>
                    {move || {
                        let syncing = state.action.get() == Some("sync");
                        let status = state.status.get();
                        let connection_state = if syncing {
                            SyncConnectionState::Syncing
                        } else {
                            status.as_ref().map(|status| status.state).unwrap_or(SyncConnectionState::Disabled)
                        };
                        view! {
                            <span class=format!("sync-badge {}", status_class(connection_state))>
                                {status_label(connection_state)}
                            </span>
                        }
                    }}
                </div>
                {move || state.status.get().map(|status| view! {
                    <dl class="sync-details">
                        <div><dt>"Device name"</dt><dd>{status.device_name}</dd></div>
                        <div><dt>"Unsynced events"</dt><dd class="sync-count">{status.unsynced_event_count}</dd></div>
                        <div><dt>"Server"</dt><dd>{status.server_url.unwrap_or_else(|| "Not configured".into())}</dd></div>
                        <div><dt>"Last attempted"</dt><dd>{format_sync_time(status.last_attempted_sync_at)}</dd></div>
                        <div><dt>"Last successful"</dt><dd>{format_sync_time(status.last_successful_sync_at)}</dd></div>
                    </dl>
                    <details class="device-id">
                        <summary>"Device ID"</summary>
                        <code>{status.device_id}</code>
                    </details>
                })}
                {move || state.status.get().and_then(|status| status.last_error).map(|error| view! {
                    <div class="sync-error-box">
                        <strong>"Last sync error"</strong>
                        <p>{error}</p>
                        <button
                            class="ghost"
                            disabled=move || state.action.get().is_some()
                            on:click=move |_| {
                                state.start_action("clear");
                                spawn_local(async move {
                                    match api::clear_sync_error().await {
                                        Ok(status) => {
                                            state.status.set(Some(status));
                                            state.notice.set(Some("Sync error cleared.".into()));
                                        }
                                        Err(error) => state.error.set(Some(format!("Could not clear sync error: {error}"))),
                                    }
                                    state.finish_action();
                                });
                            }
                        >{move || if state.action.get() == Some("clear") { "Clearing..." } else { "Clear error" }}</button>
                    </div>
                })}
                <div class="sync-actions">
                    <button
                        class="ghost"
                        disabled=move || state.action.get().is_some()
                        on:click=move |_| {
                            state.start_action("test");
                            spawn_local(async move {
                                match api::test_sync_connection().await {
                                    Ok(result) => {
                                        let server = result.server_name.unwrap_or_else(|| "OpenMgmt server".into());
                                        state.notice.set(Some(format!(
                                            "Connection successful: {server}, protocol {}.",
                                            result.protocol_version
                                        )));
                                    }
                                    Err(error) => state.error.set(Some(format!("Connection test failed: {error}"))),
                                }
                                state.reload_status().await;
                                state.finish_action();
                            });
                        }
                    >{move || if state.action.get() == Some("test") { "Testing..." } else { "Test connection" }}</button>
                    <button
                        class="primary"
                        disabled=move || state.action.get().is_some()
                        on:click=move |_| {
                            state.start_action("sync");
                            spawn_local(async move {
                                match api::sync_now().await {
                                    Ok(result) => {
                                        state.notice.set(Some(sync_result_summary(&result)));
                                        state.result.set(Some(result));
                                    }
                                    Err(error) => state.error.set(Some(format!("Sync failed: {error}"))),
                                }
                                state.reload_status().await;
                                state.finish_action();
                            });
                        }
                    >{move || if state.action.get() == Some("sync") { "Syncing..." } else { "Sync now" }}</button>
                </div>
            </section>
        </section>

        {move || state.result.get().map(|result| view! {
            <section class="panel">
                <div class="section-title"><div><p class="eyebrow">LATEST RUN</p><h2>"Sync result"</h2></div></div>
                <div class="sync-result">
                    <div><span>"Pushed"</span><strong>{result.pushed_event_count}</strong></div>
                    <div><span>"Accepted"</span><strong>{result.accepted_event_count}</strong></div>
                    <div><span>"Rejected"</span><strong>{result.rejected_event_count}</strong></div>
                    <div><span>"Pulled"</span><strong>{result.pulled_event_count}</strong></div>
                    <div><span>"Applied"</span><strong>{result.applied_event_count}</strong></div>
                    <div><span>"Conflicts"</span><strong>{result.conflict_count}</strong></div>
                </div>
                {result.server_checkpoint.map(|checkpoint| view! {
                    <p class="checkpoint"><span>"Checkpoint"</span><code>{checkpoint}</code></p>
                })}
            </section>
        })}
    }
}

fn status_class(state: SyncConnectionState) -> &'static str {
    match state {
        SyncConnectionState::Disabled => "disabled",
        SyncConnectionState::NotConfigured => "not-configured",
        SyncConnectionState::Ready => "ready",
        SyncConnectionState::Syncing => "syncing",
        SyncConnectionState::Error => "error",
    }
}

#[component]
fn Dashboard(state: AppState) -> impl IntoView {
    view! {
        <Header eyebrow="COMMAND CENTER" title="Keep the important work moving." description="A local view across every organization, project, and task." />
        <section class="metrics">
            <Metric label="Organizations" value=Signal::derive(move || state.snapshot.get().organizations.len()) />
            <Metric label="Active projects" value=Signal::derive(move || state.snapshot.get().projects.iter().filter(|p| p.status == ProjectStatus::Active).count()) />
            <Metric label="Open tasks" value=Signal::derive(move || state.snapshot.get().tasks.iter().filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Canceled)).count()) />
            <Metric label="Needs attention" value=Signal::derive(move || { let board=state.snapshot.get().board; board.overdue.len()+board.waiting_blocked.len() }) />
        </section>
        <section class="panel">
            <div class="section-title"><div><p class="eyebrow">FOCUS QUEUE</p><h2>Highest urgency</h2></div><button class="ghost" on:click=move |_| state.refresh()>"Refresh"</button></div>
            <div class="task-list">
                {move || {
                    let board=state.snapshot.get().board;
                    let tasks=board.now.into_iter().chain(board.overdue).chain(board.due_soon).take(7).collect::<Vec<_>>();
                    if tasks.is_empty() {
                        view! { <div class="empty">"No urgent tasks. Create a task to populate the queue."</div> }.into_any()
                    } else {
                        tasks.into_iter().map(|item| view! {
                            <TaskCard task=item.context.task project=item.context.project_name state />
                        }).collect_view().into_any()
                    }
                }}
            </div>
        </section>
    }
}

#[component]
fn Metric(label: &'static str, value: Signal<usize>) -> impl IntoView {
    view! { <div><span>{label}</span><strong>{move || value.get()}</strong></div> }
}

#[component]
fn OrganizationsView(state: AppState) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let description = NodeRef::<leptos::html::Input>::new();
    let color = NodeRef::<leptos::html::Input>::new();
    let icon = NodeRef::<leptos::html::Input>::new();
    view! {
        <Header eyebrow="PORTFOLIO" title="Organizations" description="Create, edit, and archive operating contexts." />
        <form class="quick-form wrap" on:submit=move |event| {
            event.prevent_default();
            let input=NewOrganization {
                name: input_value(name),
                slug: None,
                description: optional_text(input_value(description)),
                color: optional_text(input_value(color)),
                icon: optional_text(input_value(icon)),
            };
            spawn_local(async move {
                finish_action(
                    state,
                    invoke::<Organization>("create_organization",serde_json::json!({"input":input})).await,
                    "Organization created.",
                    "Create organization failed",
                ).await;
            });
        }>
            <input node_ref=name placeholder="Organization name" required />
            <input node_ref=description placeholder="Description" />
            <input node_ref=color type="color" value="#52725a" aria-label="Color" />
            <input node_ref=icon placeholder="Icon initials" maxlength="4" />
            <button class="primary">"Create organization"</button>
        </form>
        <section class="cards">
            {move || {
                let organizations=state.snapshot.get().organizations;
                if organizations.is_empty() {
                    view! { <div class="empty">"No organizations yet."</div> }.into_any()
                } else {
                    organizations.into_iter().map(|organization| view! {
                        <OrganizationCard organization state />
                    }).collect_view().into_any()
                }
            }}
        </section>
    }
}

#[component]
fn OrganizationCard(organization: Organization, state: AppState) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let description = NodeRef::<leptos::html::Textarea>::new();
    let color = NodeRef::<leptos::html::Input>::new();
    let icon = NodeRef::<leptos::html::Input>::new();
    let organization_id = organization.id.clone();
    let archive_id = organization.id.clone();
    let current_name = organization.name.clone();
    let accent = organization
        .color
        .clone()
        .unwrap_or_else(|| "#778077".into());
    let accent_input = accent.clone();
    view! {
        <article class="org-card" style=format!("--accent:{}",accent.clone())>
            <span class="org-icon">{organization.icon.clone().unwrap_or_else(|| organization.name.chars().take(2).collect())}</span>
            <h2>{organization.name.clone()}</h2>
            <p>{organization.description.clone().unwrap_or_else(|| "No description yet.".into())}</p>
            <small>{format!("/{}",organization.slug)}</small>
            <details class="editor">
                <summary>"Edit organization"</summary>
                <form on:submit=move |event| {
                    event.prevent_default();
                    let patch=OrganizationPatch {
                        name: Some(input_value(name)),
                        slug: None,
                        description: Some(optional_text(textarea_value(description))),
                        color: Some(optional_text(input_value(color))),
                        icon: Some(optional_text(input_value(icon))),
                    };
                    let id=organization_id.clone();
                    spawn_local(async move {
                        finish_action(
                            state,
                            invoke::<Organization>("update_organization",serde_json::json!({"id":id,"patch":patch})).await,
                            "Organization updated.",
                            "Update organization failed",
                        ).await;
                    });
                }>
                    <label>"Name"<input node_ref=name value=organization.name required /></label>
                    <label>"Description"<textarea node_ref=description>{organization.description.unwrap_or_default()}</textarea></label>
                    <label>"Color"<input node_ref=color type="color" value=accent_input /></label>
                    <label>"Icon"<input node_ref=icon value=organization.icon.unwrap_or_default() maxlength="4" /></label>
                    <div class="form-actions">
                        <button class="primary small">"Save"</button>
                        <button type="button" class="danger small" on:click=move |_| {
                            if !confirmed(&format!("Archive {current_name}? Its projects and tasks will be hidden.")) { return; }
                            let id=archive_id.clone();
                            spawn_local(async move {
                                finish_action(
                                    state,
                                    invoke::<()>("archive_organization",serde_json::json!({"id":id})).await,
                                    "Organization archived.",
                                    "Archive organization failed",
                                ).await;
                            });
                        }>"Archive"</button>
                    </div>
                </form>
            </details>
        </article>
    }
}

#[component]
fn ProjectsView(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let organization = NodeRef::<leptos::html::Select>::new();
    let description = NodeRef::<leptos::html::Input>::new();
    let project_type = NodeRef::<leptos::html::Select>::new();
    let priority = NodeRef::<leptos::html::Select>::new();
    view! {
        <Header eyebrow="PORTFOLIO" title="Projects" description="Create, edit, archive, and open active bodies of work." />
        <form class="quick-form wrap" on:submit=move |event| {
            event.prevent_default();
            let parsed_type=select_value(project_type).parse().unwrap_or(ProjectType::Other);
            let input=NewProject {
                organization_id: select_value(organization),
                name: input_value(name),
                slug: None,
                description: optional_text(input_value(description)),
                project_type: parsed_type,
                status: ProjectStatus::Active,
                priority: parse_i32(select_value(priority)).unwrap_or(3),
                deadline: None,
                repo_url: None,
                notes: None,
            };
            spawn_local(async move {
                finish_action(
                    state,
                    invoke::<Project>("create_project",serde_json::json!({"input":input})).await,
                    "Project created.",
                    "Create project failed",
                ).await;
            });
        }>
            <input node_ref=name placeholder="Project name" required />
            <select node_ref=organization required><option value="">"Organization"</option>{move || state.snapshot.get().organizations.into_iter().map(|item|view!{<option value=item.id>{item.name}</option>}).collect_view()}</select>
            <input node_ref=description placeholder="Description" />
            <select node_ref=project_type>{project_type_options().into_iter().map(|(value,label)|view!{<option value=value.to_string()>{label}</option>}).collect_view()}</select>
            <select node_ref=priority>{(1..=5).map(|value|view!{<option value=value selected=value==3>{format!("Priority {value}")}</option>}).collect_view()}</select>
            <button class="primary">"Create project"</button>
        </form>
        <section class="cards">
            {move || {
                let projects=state.snapshot.get().projects;
                if projects.is_empty() {
                    view! { <div class="empty">"No projects yet."</div> }.into_any()
                } else {
                    projects.into_iter().map(|project| view! {
                        <ProjectCard project state page />
                    }).collect_view().into_any()
                }
            }}
        </section>
    }
}

#[component]
fn ProjectCard(project: Project, state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let project_id = project.id.clone();
    let open_id = project.id.clone();
    view! {
        <article class="project-card">
            <div class="card-meta"><Status value=project.status.to_string() /><Priority value=project.priority /></div>
            <h2>{project.name.clone()}</h2>
            <p>{project.description.clone().unwrap_or_else(|| "No description yet.".into())}</p>
            <small>{project.project_type.to_string()}</small>
            <div class="form-actions">
                <button class="ghost" on:click=move |_| page.set(Page::Project(open_id.clone()))>"Open"</button>
            </div>
            <ProjectEditor project state project_id />
        </article>
    }
}

#[component]
fn ProjectEditor(project: Project, state: AppState, project_id: String) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let description = NodeRef::<leptos::html::Textarea>::new();
    let project_type = NodeRef::<leptos::html::Select>::new();
    let status = NodeRef::<leptos::html::Select>::new();
    let priority = NodeRef::<leptos::html::Select>::new();
    let deadline = NodeRef::<leptos::html::Input>::new();
    let repo_url = NodeRef::<leptos::html::Input>::new();
    let notes = NodeRef::<leptos::html::Textarea>::new();
    let archive_id = project.id.clone();
    let project_name = project.name.clone();
    let current_type = project.project_type;
    let current_status = project.status;
    let current_priority = project.priority;
    view! {
        <details class="editor">
            <summary>"Edit project"</summary>
            <form on:submit=move |event| {
                event.prevent_default();
                let deadline_value=match parse_datetime_local(input_value(deadline)) {
                    Ok(value)=>value,
                    Err(error)=>{state.fail("Update project failed",error);return;}
                };
                let patch=ProjectPatch {
                    name: Some(input_value(name)),
                    slug: None,
                    description: Some(optional_text(textarea_value(description))),
                    project_type: select_value(project_type).parse().ok(),
                    status: select_value(status).parse().ok(),
                    priority: parse_i32(select_value(priority)),
                    deadline: Some(deadline_value),
                    repo_url: Some(optional_text(input_value(repo_url))),
                    notes: Some(optional_text(textarea_value(notes))),
                };
                let id=project_id.clone();
                spawn_local(async move {
                    finish_action(
                        state,
                        invoke::<Project>("update_project",serde_json::json!({"id":id,"patch":patch})).await,
                        "Project updated.",
                        "Update project failed",
                    ).await;
                });
            }>
                <label>"Name"<input node_ref=name value=project.name required /></label>
                <label>"Description"<textarea node_ref=description>{project.description.unwrap_or_default()}</textarea></label>
                <label>"Type"<select node_ref=project_type>{project_type_options().into_iter().map(|(value,label)|view!{<option value=value.to_string() selected=value==current_type>{label}</option>}).collect_view()}</select></label>
                <label>"Status"<select node_ref=status>{project_status_options().into_iter().map(|(value,label)|view!{<option value=value.to_string() selected=value==current_status>{label}</option>}).collect_view()}</select></label>
                <label>"Priority"<select node_ref=priority>{(1..=5).map(|value|view!{<option value=value selected=value==current_priority>{value}</option>}).collect_view()}</select></label>
                <label>"Deadline"<input node_ref=deadline type="datetime-local" value=datetime_local_value(project.deadline) /></label>
                <label>"Repository URL"<input node_ref=repo_url value=project.repo_url.unwrap_or_default() /></label>
                <label>"Notes"<textarea node_ref=notes>{project.notes.unwrap_or_default()}</textarea></label>
                <div class="form-actions">
                    <button class="primary small">"Save"</button>
                    <button type="button" class="danger small" on:click=move |_| {
                        if !confirmed(&format!("Archive project {project_name}? Its tasks will be hidden.")) { return; }
                        let id=archive_id.clone();
                        spawn_local(async move {
                            finish_action(
                                state,
                                invoke::<()>("archive_project",serde_json::json!({"id":id})).await,
                                "Project archived.",
                                "Archive project failed",
                            ).await;
                        });
                    }>"Archive"</button>
                </div>
            </form>
        </details>
    }
}

#[component]
fn ProjectView(id: String, state: AppState) -> impl IntoView {
    let header_id = id.clone();
    let tasks_id = id;
    view! {
        {move || {
            match state.snapshot.get().projects.into_iter().find(|project|project.id==header_id) {
                Some(project)=>view!{
                    <header class="page-header"><div><p class="eyebrow">{project.project_type.to_string()}</p><h1>{project.name.clone()}</h1><p>{project.description.clone().unwrap_or_else(||"No project description.".into())}</p></div></header>
                    <section class="panel compact"><ProjectEditor project=project.clone() state project_id=project.id /></section>
                }.into_any(),
                None=>view!{<div class="empty">"Project not found or archived."</div>}.into_any(),
            }
        }}
        <section class="panel"><div class="section-title"><h2>"Project tasks"</h2><button class="ghost" on:click=move |_|state.refresh()>"Refresh"</button></div><div class="task-list">
            {move || {
                let tasks=state.snapshot.get().tasks.into_iter().filter(|task|task.project_id==tasks_id).collect::<Vec<_>>();
                if tasks.is_empty() {
                    view!{<div class="empty">"No active tasks in this project."</div>}.into_any()
                } else {
                    tasks.into_iter().map(|task|view!{<TaskCard task state />}).collect_view().into_any()
                }
            }}
        </div></section>
    }
}

#[component]
fn TasksView(state: AppState) -> impl IntoView {
    let title = NodeRef::<leptos::html::Input>::new();
    let project = NodeRef::<leptos::html::Select>::new();
    let priority = NodeRef::<leptos::html::Select>::new();
    let due_at = NodeRef::<leptos::html::Input>::new();
    view! {
        <Header eyebrow="EXECUTION" title="Tasks" description="Create, edit, start, block, complete, and cancel work." />
        <form class="quick-form wrap" on:submit=move |event| {
            event.prevent_default();
            let due=match parse_datetime_local(input_value(due_at)) {
                Ok(value)=>value,
                Err(error)=>{state.fail("Create task failed",error);return;}
            };
            let input=NewTask {
                project_id: select_value(project),
                title: input_value(title),
                description: None,
                status: TaskStatus::Inbox,
                priority: parse_i32(select_value(priority)).unwrap_or(3),
                due_at: due,
                scheduled_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                tags: vec![],
            };
            spawn_local(async move {
                finish_action(
                    state,
                    invoke::<Task>("create_task",serde_json::json!({"input":input})).await,
                    "Task created.",
                    "Create task failed",
                ).await;
            });
        }>
            <input node_ref=title placeholder="Task title" required />
            <select node_ref=project required><option value="">"Project"</option>{move ||state.snapshot.get().projects.into_iter().map(|item|view!{<option value=item.id>{item.name}</option>}).collect_view()}</select>
            <select node_ref=priority>{(1..=5).map(|value|view!{<option value=value selected=value==3>{format!("Priority {value}")}</option>}).collect_view()}</select>
            <input node_ref=due_at type="datetime-local" aria-label="Due date" />
            <button class="primary">"Create task"</button>
        </form>
        <section class="panel task-list">
            {move || {
                let snapshot=state.snapshot.get();
                if snapshot.tasks.is_empty() {
                    view!{<div class="empty">"No active tasks. Create one above or seed the database."</div>}.into_any()
                } else {
                    snapshot.tasks.into_iter().map(|task|{
                        let project=snapshot.projects.iter().find(|project|project.id==task.project_id).map(|project|project.name.clone()).unwrap_or_default();
                        view!{<TaskCard task project state />}
                    }).collect_view().into_any()
                }
            }}
        </section>
    }
}

#[component]
fn TodayView(board: Signal<BoardState>, state: AppState) -> impl IntoView {
    view! {
        <Header eyebrow="DAILY PLAN" title="Today" description="The work that matters now, ordered by urgency and context." />
        <div class="section-toolbar"><button class="ghost" on:click=move |_|state.refresh()>"Refresh board"</button></div>
        <section class="today-grid">
            <TaskGroup title="Now" tasks=Signal::derive(move ||board.get().now) state />
            <TaskGroup title="Overdue" tasks=Signal::derive(move ||board.get().overdue) state />
            <TaskGroup title="Due soon" tasks=Signal::derive(move ||board.get().due_soon) state />
            <TaskGroup title="Later today" tasks=Signal::derive(move ||board.get().later_today) state />
            <TaskGroup title="Waiting / blocked" tasks=Signal::derive(move ||board.get().waiting_blocked) state />
            <TaskGroup title="Done today" tasks=Signal::derive(move ||board.get().done_today) state />
        </section>
    }
}

#[component]
fn TaskGroup(
    title: &'static str,
    tasks: Signal<Vec<ScoredTask>>,
    state: AppState,
) -> impl IntoView {
    view! {
        <section class="panel"><div class="section-title"><h2>{title}</h2><span>{move ||tasks.get().len()}</span></div><div class="task-list">
            {move || {
                let items=tasks.get();
                if items.is_empty() {
                    view!{<div class="empty small-empty">"Clear"</div>}.into_any()
                } else {
                    items.into_iter().map(|item|view!{<TaskCard task=item.context.task project=item.context.project_name state />}).collect_view().into_any()
                }
            }}
        </div></section>
    }
}

#[component]
fn TaskCard(
    task: Task,
    #[prop(default = String::new())] project: String,
    #[prop(optional)] state: Option<AppState>,
) -> impl IntoView {
    let status = task.status;
    let elapsed = task
        .started_at
        .map(|at| (Utc::now() - at).num_minutes().max(0));
    let time_limit = task.time_limit_minutes;
    view! {
        <article class="task-card">
            <span class="task-dot">{task.priority}</span>
            <div class="task-summary">
                <strong>{task.title.clone()}</strong>
                <p>
                    <Status value=status.to_string() />
                    {(!project.is_empty()).then(||view!{<span>{project}</span>})}
                    {elapsed.map(|minutes|view!{<span class="timer">{format!("{minutes}m active")}</span>})}
                    {time_limit.map(|minutes|view!{<span>{format!("limit {minutes}m")}</span>})}
                    {task.pinned.then(||view!{<span>"pinned"</span>})}
                </p>
            </div>
            {state.map(|state|view!{<TaskActions task=task.clone() state />})}
        </article>
    }
}

#[component]
fn TaskActions(task: Task, state: AppState) -> impl IntoView {
    let start_id = task.id.clone();
    let done_id = task.id.clone();
    let block_id = task.id.clone();
    let unblock_id = task.id.clone();
    let cancel_id = task.id.clone();
    let cancel_title = task.title.clone();
    let block_reason = task
        .blocked_reason
        .clone()
        .unwrap_or_else(|| "Blocked from task list".into());
    let can_start = !matches!(
        task.status,
        TaskStatus::InProgress | TaskStatus::Done | TaskStatus::Canceled
    );
    let can_complete = !matches!(task.status, TaskStatus::Done | TaskStatus::Canceled);
    view! {
        <div class="task-controls">
            <div class="actions">
                {can_start.then(||view!{<button class="ghost" on:click=move |_|{
                    let id=start_id.clone();
                    spawn_local(async move {
                        finish_action(state,invoke::<Task>("start_task",serde_json::json!({"id":id})).await,"Task started.","Start task failed").await;
                    });
                }>"Start"</button>})}
                {can_complete.then(||view!{<button class="primary small" on:click=move |_|{
                    let id=done_id.clone();
                    spawn_local(async move {
                        finish_action(state,invoke::<Task>("complete_task",serde_json::json!({"id":id})).await,"Task completed.","Complete task failed").await;
                    });
                }>"Done"</button>})}
                {matches!(task.status,TaskStatus::Blocked|TaskStatus::Waiting).then(||view!{<button class="ghost" on:click=move |_|{
                    let id=unblock_id.clone();
                    spawn_local(async move {
                        finish_action(state,invoke::<Task>("unblock_task",serde_json::json!({"id":id})).await,"Task unblocked.","Unblock task failed").await;
                    });
                }>"Unblock"</button>})}
                {(!matches!(task.status,TaskStatus::Blocked|TaskStatus::Waiting|TaskStatus::Done|TaskStatus::Canceled)).then(||view!{<button class="ghost" on:click=move |_|{
                    let id=block_id.clone();
                    let reason=block_reason.clone();
                    spawn_local(async move {
                        finish_action(state,invoke::<Task>("block_task",serde_json::json!({"id":id,"reason":reason})).await,"Task blocked.","Block task failed").await;
                    });
                }>"Block"</button>})}
            </div>
            <TaskEditor task=task.clone() state />
            <button class="danger-link" on:click=move |_|{
                if !confirmed(&format!("Cancel task {cancel_title}?")) { return; }
                let id=cancel_id.clone();
                spawn_local(async move {
                    finish_action(state,invoke::<Task>("cancel_task",serde_json::json!({"id":id})).await,"Task canceled.","Cancel task failed").await;
                });
            }>"Cancel"</button>
        </div>
    }
}

#[component]
fn TaskEditor(task: Task, state: AppState) -> impl IntoView {
    let title = NodeRef::<leptos::html::Input>::new();
    let description = NodeRef::<leptos::html::Textarea>::new();
    let status = NodeRef::<leptos::html::Select>::new();
    let priority = NodeRef::<leptos::html::Select>::new();
    let due_at = NodeRef::<leptos::html::Input>::new();
    let scheduled_at = NodeRef::<leptos::html::Input>::new();
    let estimated = NodeRef::<leptos::html::Input>::new();
    let time_limit = NodeRef::<leptos::html::Input>::new();
    let pinned = NodeRef::<leptos::html::Input>::new();
    let blocked_reason = NodeRef::<leptos::html::Input>::new();
    let tags = NodeRef::<leptos::html::Input>::new();
    let task_id = task.id.clone();
    let current_status = task.status;
    let current_priority = task.priority;
    view! {
        <details class="editor task-editor">
            <summary>"Edit"</summary>
            <form on:submit=move |event|{
                event.prevent_default();
                let due=match parse_datetime_local(input_value(due_at)){Ok(value)=>value,Err(error)=>{state.fail("Update task failed",error);return;}};
                let scheduled=match parse_datetime_local(input_value(scheduled_at)){Ok(value)=>value,Err(error)=>{state.fail("Update task failed",error);return;}};
                let patch=TaskPatch {
                    title: Some(input_value(title)),
                    description: Some(optional_text(textarea_value(description))),
                    status: select_value(status).parse().ok(),
                    priority: parse_i32(select_value(priority)),
                    due_at: Some(due),
                    scheduled_at: Some(scheduled),
                    estimated_minutes: Some(parse_i32(input_value(estimated))),
                    time_limit_minutes: Some(parse_i32(input_value(time_limit))),
                    pinned: Some(pinned.get().is_some_and(|input|input.checked())),
                    blocked_reason: Some(optional_text(input_value(blocked_reason))),
                    tags: Some(input_value(tags).split(',').map(str::trim).filter(|tag|!tag.is_empty()).map(str::to_owned).collect()),
                };
                let id=task_id.clone();
                spawn_local(async move {
                    finish_action(
                        state,
                        invoke::<Task>("update_task",serde_json::json!({"id":id,"patch":patch})).await,
                        "Task updated.",
                        "Update task failed",
                    ).await;
                });
            }>
                <label>"Title"<input node_ref=title value=task.title required /></label>
                <label>"Description"<textarea node_ref=description>{task.description.unwrap_or_default()}</textarea></label>
                <label>"Status"<select node_ref=status>{task_status_options().into_iter().map(|(value,label)|view!{<option value=value.to_string() selected=value==current_status>{label}</option>}).collect_view()}</select></label>
                <label>"Priority"<select node_ref=priority>{(1..=5).map(|value|view!{<option value=value selected=value==current_priority>{value}</option>}).collect_view()}</select></label>
                <label>"Due"<input node_ref=due_at type="datetime-local" value=datetime_local_value(task.due_at) /></label>
                <label>"Scheduled"<input node_ref=scheduled_at type="datetime-local" value=datetime_local_value(task.scheduled_at) /></label>
                <label>"Estimated minutes"<input node_ref=estimated type="number" min="1" value=task.estimated_minutes.map(|value|value.to_string()).unwrap_or_default() /></label>
                <label>"Time limit minutes"<input node_ref=time_limit type="number" min="1" value=task.time_limit_minutes.map(|value|value.to_string()).unwrap_or_default() /></label>
                <label class="checkbox"><input node_ref=pinned type="checkbox" checked=task.pinned />"Pinned"</label>
                <label>"Blocked reason"<input node_ref=blocked_reason value=task.blocked_reason.unwrap_or_default() /></label>
                <label>"Tags (comma separated)"<input node_ref=tags value=task.tags.join(", ") /></label>
                <button class="primary small">"Save task"</button>
            </form>
        </details>
    }
}

#[component]
fn Status(value: String) -> impl IntoView {
    view! { <span class="status">{value.replace('_'," ")}</span> }
}

#[component]
fn Priority(value: i32) -> impl IntoView {
    view! { <span class=format!("priority p{value}")>{format!("P{value}")}</span> }
}

fn board_task_count(board: &BoardState) -> usize {
    board.now.len()
        + board.next_up.len()
        + board.due_soon.len()
        + board.waiting_blocked.len()
        + board.later_today.len()
        + board.overdue.len()
        + board.done_today.len()
}

#[component]
fn BoardView(
    board: Signal<BoardState>,
    error: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    now: RwSignal<DateTime<Utc>>,
    state: AppState,
) -> impl IntoView {
    view! {
        <main class="board">
            <header class="board-header">
                <div class="board-brand"><span>OM</span><div><strong>OPENMGMT</strong><small>LIVE OPERATIONS BOARD</small></div></div>
                <p>{move ||now.get().format("%A, %B %-d").to_string()}</p>
                <time>{move ||now.get().format("%-I:%M:%S %p").to_string()}</time>
            </header>
            {move ||error.get().map(|message|view!{<div class="board-message error">{message}</div>})}
            {move ||loading.get().then(||view!{<div class="board-message">"Refreshing board..."</div>})}
            {move ||(!loading.get() && error.get().is_none() && board_task_count(&board.get())==0).then(||view!{
                <div class="board-empty"><h2>"No board tasks"</h2><p>"Create an active task in the main window."</p></div>
            })}
            <section class="board-columns">
                <BoardColumn title="NOW" tasks=Signal::derive(move ||board.get().now) class="now" />
                <BoardColumn title="NEXT UP" tasks=Signal::derive(move ||board.get().next_up) class="next" />
                <BoardColumn title="DUE SOON" tasks=Signal::derive(move ||board.get().due_soon) class="due" />
                <BoardColumn title="WAITING / BLOCKED" tasks=Signal::derive(move ||board.get().waiting_blocked) class="waiting" />
                <BoardColumn title="LATER TODAY" tasks=Signal::derive(move ||board.get().later_today) class="later" />
                <BoardColumn title="OVERDUE" tasks=Signal::derive(move ||board.get().overdue) class="overdue" />
                <BoardColumn title="DONE TODAY" tasks=Signal::derive(move ||board.get().done_today) class="done" />
            </section>
            <footer><span><i></i>" Auto-refreshing every 10 seconds"</span><button on:click=move |_|state.refresh()>"Refresh now"</button><span>"LOCAL / PRIVATE / LIVE"</span></footer>
        </main>
    }
}

#[component]
fn BoardColumn(
    title: &'static str,
    tasks: Signal<Vec<ScoredTask>>,
    class: &'static str,
) -> impl IntoView {
    view! {
        <section class=format!("board-column {class}")>
            <header><h2>{title}</h2><span>{move ||tasks.get().len()}</span></header>
            <div>
                {move || {
                    let items=tasks.get();
                    if items.is_empty() {
                        view!{<p class="column-empty">"No tasks"</p>}.into_any()
                    } else {
                        items.into_iter().map(|item|view!{<BoardCard item />}).collect_view().into_any()
                    }
                }}
            </div>
        </section>
    }
}

#[component]
fn BoardCard(item: ScoredTask) -> impl IntoView {
    let task = item.context.task;
    view! {
        <article class="board-card" style=format!("--accent:{}",item.context.organization_color.unwrap_or_else(||"#95a095".into()))>
            <div class="board-meta"><Priority value=task.priority /><span>{item.context.organization_name}</span>{task.pinned.then(||view!{<b>"PINNED"</b>})}</div>
            <h3>{task.title}</h3><p>{format!("{} / {}",item.context.project_name,item.context.project_type)}</p>
            <div class="board-bottom">
                {task.time_limit_minutes.map(|minutes|view!{<span>{format!("LIMIT {minutes}M")}</span>})}
                {task.due_at.map(|at|view!{<span>{at.format("%-I:%M %p").to_string()}</span>})}
                {task.blocked_reason.map(|reason|view!{<span>{reason}</span>})}
            </div>
        </article>
    }
}
