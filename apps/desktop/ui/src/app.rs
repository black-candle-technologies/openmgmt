use chrono::{DateTime, Utc};
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use openmgmt_core::{
    BoardState, NewProject, NewTask, Organization, Project, ProjectStatus, ProjectType, ScoredTask,
    Task, TaskStatus,
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
    Board,
}

#[derive(Clone, Default)]
struct Snapshot {
    organizations: Vec<Organization>,
    projects: Vec<Project>,
    tasks: Vec<Task>,
    board: BoardState,
}

async fn invoke<T: DeserializeOwned>(command: &str, args: impl Serialize) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|error| error.to_string())?;
    let value = JsFuture::from(tauri_invoke(command, args))
        .await
        .map_err(|error| format!("{error:?}"))?;
    serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
}

async fn load_snapshot() -> Result<Snapshot, String> {
    Ok(Snapshot {
        organizations: invoke("list_organizations", ()).await?,
        projects: invoke("list_projects", ()).await?,
        tasks: invoke("list_tasks", ()).await?,
        board: invoke("get_board_state", ()).await?,
    })
}

#[component]
pub fn App() -> impl IntoView {
    let board_only = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .is_some_and(|query| query.contains("board=1"));
    let page = RwSignal::new(if board_only {
        Page::Board
    } else {
        Page::Dashboard
    });
    let snapshot = RwSignal::new(Snapshot::default());
    let error = RwSignal::new(None::<String>);
    let now = RwSignal::new(Utc::now());

    let refresh = Callback::new(move |_: ()| {
        spawn_local(async move {
            match load_snapshot().await {
                Ok(value) => {
                    snapshot.set(value);
                    error.set(None);
                }
                Err(message) => error.set(Some(message)),
            }
        });
    });
    refresh.run(());
    let interval = Interval::new(10_000, move || {
        now.set(Utc::now());
        refresh.run(());
    });
    interval.forget();

    if board_only {
        return view! { <BoardView board=Signal::derive(move || snapshot.get().board) now /> }
            .into_any();
    }

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
                </nav>
                <button class="primary tv" on:click=move |_| {
                    spawn_local(async move {
                        let _: Result<(), _> = invoke("open_tv_board_window", ()).await;
                    });
                }>"Open TV Board ↗"</button>
                <p class="local"><i></i>" Local database"</p>
            </aside>
            <main class="content">
                {move || error.get().map(|message| view! { <div class="error">{message}</div> })}
                {move || match page.get() {
                    Page::Dashboard => view! { <Dashboard snapshot /> }.into_any(),
                    Page::Organizations => view! { <OrganizationsView snapshot refresh /> }.into_any(),
                    Page::Projects => view! { <ProjectsView snapshot page refresh /> }.into_any(),
                    Page::Project(id) => view! { <ProjectView id snapshot refresh /> }.into_any(),
                    Page::Tasks => view! { <TasksView snapshot refresh /> }.into_any(),
                    Page::Today => view! { <TodayView board=Signal::derive(move || snapshot.get().board) refresh /> }.into_any(),
                    Page::Board => view! { <BoardView board=Signal::derive(move || snapshot.get().board) now /> }.into_any(),
                }}
            </main>
        </div>
    }.into_any()
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

#[component]
fn Dashboard(snapshot: RwSignal<Snapshot>) -> impl IntoView {
    view! {
        <Header eyebrow="COMMAND CENTER" title="Keep the important work moving." description="A local view across every organization, project, and task." />
        <section class="metrics">
            <Metric label="Organizations" value=Signal::derive(move || snapshot.get().organizations.len()) />
            <Metric label="Active projects" value=Signal::derive(move || snapshot.get().projects.iter().filter(|p| p.status == ProjectStatus::Active).count()) />
            <Metric label="Open tasks" value=Signal::derive(move || snapshot.get().tasks.iter().filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Canceled)).count()) />
            <Metric label="Needs attention" value=Signal::derive(move || { let b=snapshot.get().board; b.overdue.len()+b.waiting_blocked.len() }) />
        </section>
        <section class="panel">
            <div class="section-title"><div><p class="eyebrow">FOCUS QUEUE</p><h2>Highest urgency</h2></div></div>
            <div class="task-list">
                {move || {
                    let board=snapshot.get().board;
                    board.now.into_iter().chain(board.overdue).chain(board.due_soon).take(7)
                        .map(|task| view! { <TaskCard task=task.context.task project=task.context.project_name /> }).collect_view()
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
fn OrganizationsView(snapshot: RwSignal<Snapshot>, refresh: Callback<()>) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    view! {
        <Header eyebrow="PORTFOLIO" title="Organizations" description="Group work by company, publication, initiative, or personal context." />
        <form class="quick-form" on:submit=move |event| {
            event.prevent_default();
            let value=name.get().map(|input| input.value()).unwrap_or_default();
            if value.trim().is_empty() { return; }
            spawn_local(async move {
                let _: Result<Organization,_> = invoke("create_organization", serde_json::json!({"input":{"name":value,"slug":null,"description":null,"color":"#52725a","icon":null}})).await;
                refresh.run(());
            });
        }>
            <input node_ref=name placeholder="New organization name" />
            <button class="primary">Create organization</button>
        </form>
        <section class="cards">
            {move || snapshot.get().organizations.into_iter().map(|org| view! {
                <article class="org-card" style=format!("--accent:{}",org.color.clone().unwrap_or("#778077".into()))>
                    <span class="org-icon">{org.icon.unwrap_or_else(|| org.name.chars().take(2).collect())}</span>
                    <h2>{org.name}</h2><p>{org.description.unwrap_or("No description yet.".into())}</p><small>{format!("/{}",org.slug)}</small>
                </article>
            }).collect_view()}
        </section>
    }
}

#[component]
fn ProjectsView(
    snapshot: RwSignal<Snapshot>,
    page: RwSignal<Page>,
    refresh: Callback<()>,
) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let org = NodeRef::<leptos::html::Select>::new();
    view! {
        <Header eyebrow="PORTFOLIO" title="Projects" description="Active bodies of work, grouped by organization and type." />
        <form class="quick-form" on:submit=move |event| {
            event.prevent_default();
            let project_name=name.get().map(|v|v.value()).unwrap_or_default();
            let organization_id=org.get().map(|v|v.value()).unwrap_or_default();
            if project_name.trim().is_empty() || organization_id.is_empty() { return; }
            spawn_local(async move {
                let input=NewProject{organization_id,name:project_name,slug:None,description:None,project_type:ProjectType::Other,status:ProjectStatus::Active,priority:3,deadline:None,repo_url:None,notes:None};
                let _: Result<Project,_>=invoke("create_project",serde_json::json!({"input":input})).await;
                refresh.run(());
            });
        }>
            <input node_ref=name placeholder="New project name" />
            <select node_ref=org><option value="">"Organization"</option>{move || snapshot.get().organizations.into_iter().map(|o|view!{<option value=o.id>{o.name}</option>}).collect_view()}</select>
            <button class="primary">Create project</button>
        </form>
        <section class="cards">
            {move || snapshot.get().projects.into_iter().map(|project| {
                let id=project.id.clone();
                view! { <button class="project-card" on:click=move |_|page.set(Page::Project(id.clone()))>
                    <div class="card-meta"><Status value=project.status.to_string() /><Priority value=project.priority /></div>
                    <h2>{project.name}</h2><p>{project.description.unwrap_or("No description yet.".into())}</p>
                    <small>{project.project_type.to_string()}</small>
                </button> }
            }).collect_view()}
        </section>
    }
}

#[component]
fn ProjectView(id: String, snapshot: RwSignal<Snapshot>, refresh: Callback<()>) -> impl IntoView {
    let project = snapshot.get().projects.into_iter().find(|p| p.id == id);
    match project {
        Some(project) => {
            let project_id = project.id.clone();
            view! {
                <header class="page-header"><div><p class="eyebrow">{project.project_type.to_string()}</p><h1>{project.name}</h1><p>{project.description.unwrap_or("No project description.".into())}</p></div></header>
                <section class="panel"><div class="section-title"><h2>Project tasks</h2></div><div class="task-list">
                    {move || snapshot.get().tasks.into_iter().filter(|task|task.project_id==project_id).map(|task|view!{<TaskCard task refresh />}).collect_view()}
                </div></section>
            }.into_any()
        }
        None => view! { <div class="empty">Project not found.</div> }.into_any(),
    }
}

#[component]
fn TasksView(snapshot: RwSignal<Snapshot>, refresh: Callback<()>) -> impl IntoView {
    let title = NodeRef::<leptos::html::Input>::new();
    let project = NodeRef::<leptos::html::Select>::new();
    view! {
        <Header eyebrow="EXECUTION" title="Tasks" description="Capture, start, time, block, and complete work." />
        <form class="quick-form" on:submit=move |event| {
            event.prevent_default();
            let task_title=title.get().map(|v|v.value()).unwrap_or_default();
            let project_id=project.get().map(|v|v.value()).unwrap_or_default();
            if task_title.trim().is_empty() || project_id.is_empty() { return; }
            spawn_local(async move {
                let input=NewTask{project_id,title:task_title,description:None,status:TaskStatus::Inbox,priority:3,due_at:None,scheduled_at:None,estimated_minutes:None,time_limit_minutes:None,pinned:false,tags:vec![]};
                let _:Result<Task,_>=invoke("create_task",serde_json::json!({"input":input})).await;
                refresh.run(());
            });
        }>
            <input node_ref=title placeholder="New task title" />
            <select node_ref=project><option value="">"Project"</option>{move ||snapshot.get().projects.into_iter().map(|p|view!{<option value=p.id>{p.name}</option>}).collect_view()}</select>
            <button class="primary">Create task</button>
        </form>
        <section class="panel task-list">
            {move || {
                let projects=snapshot.get().projects;
                snapshot.get().tasks.into_iter().map(|task|{
                    let project=projects.iter().find(|p|p.id==task.project_id).map(|p|p.name.clone());
                    view!{<TaskCard task project=project.unwrap_or_default() refresh />}
                }).collect_view()
            }}
        </section>
    }
}

#[component]
fn TodayView(board: Signal<BoardState>, refresh: Callback<()>) -> impl IntoView {
    view! {
        <Header eyebrow="DAILY PLAN" title="Today" description="The work that matters now, ordered by urgency and context." />
        <section class="today-grid">
            <TaskGroup title="Now" tasks=Signal::derive(move ||board.get().now) refresh />
            <TaskGroup title="Overdue" tasks=Signal::derive(move ||board.get().overdue) refresh />
            <TaskGroup title="Due soon" tasks=Signal::derive(move ||board.get().due_soon) refresh />
            <TaskGroup title="Later today" tasks=Signal::derive(move ||board.get().later_today) refresh />
            <TaskGroup title="Waiting / blocked" tasks=Signal::derive(move ||board.get().waiting_blocked) refresh />
            <TaskGroup title="Done today" tasks=Signal::derive(move ||board.get().done_today) refresh />
        </section>
    }
}

#[component]
fn TaskGroup(
    title: &'static str,
    tasks: Signal<Vec<ScoredTask>>,
    refresh: Callback<()>,
) -> impl IntoView {
    view! { <section class="panel"><div class="section-title"><h2>{title}</h2><span>{move ||tasks.get().len()}</span></div><div class="task-list">
        {move ||tasks.get().into_iter().map(|item|view!{<TaskCard task=item.context.task project=item.context.project_name refresh />}).collect_view()}
    </div></section> }
}

#[component]
fn TaskCard(
    task: Task,
    #[prop(default = String::new())] project: String,
    #[prop(optional)] refresh: Option<Callback<()>>,
) -> impl IntoView {
    let id_start = task.id.clone();
    let id_done = task.id.clone();
    let active = task.status == TaskStatus::InProgress;
    let elapsed = task
        .started_at
        .map(|at| (Utc::now() - at).num_minutes().max(0));
    view! {
        <article class="task-card"><span class="task-dot">{task.priority}</span><div><strong>{task.title}</strong><p>
            <Status value=task.status.to_string() /> {(!project.is_empty()).then(||view!{<span>{project}</span>})}
            {elapsed.map(|minutes|view!{<span class="timer">{format!("{minutes}m active")}</span>})}
        </p></div>
        {refresh.map(|refresh| view! { <div class="actions">
            <button class="ghost" disabled=active on:click=move |_| {
                let id=id_start.clone(); spawn_local(async move { let _:Result<Task,_>=invoke("start_task",serde_json::json!({"id":id})).await; refresh.run(()); });
            }>"Start"</button>
            <button class="primary small" on:click=move |_| {
                let id=id_done.clone(); spawn_local(async move { let _:Result<Task,_>=invoke("complete_task",serde_json::json!({"id":id})).await; refresh.run(()); });
            }>"Done"</button>
        </div> })}
        </article>
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

#[component]
fn BoardView(board: Signal<BoardState>, now: RwSignal<DateTime<Utc>>) -> impl IntoView {
    view! {
        <main class="board">
            <header class="board-header"><div class="board-brand"><span>OM</span><div><strong>OPENMGMT</strong><small>LIVE OPERATIONS BOARD</small></div></div>
            <p>{move ||now.get().format("%A, %B %-d").to_string()}</p><time>{move ||now.get().format("%-I:%M %p").to_string()}</time></header>
            <section class="board-columns">
                <BoardColumn title="NOW" tasks=Signal::derive(move ||board.get().now) class="now" />
                <BoardColumn title="NEXT UP" tasks=Signal::derive(move ||board.get().next_up) class="next" />
                <BoardColumn title="DUE SOON" tasks=Signal::derive(move ||board.get().due_soon) class="due" />
                <BoardColumn title="WAITING / BLOCKED" tasks=Signal::derive(move ||board.get().waiting_blocked) class="waiting" />
                <BoardColumn title="LATER TODAY" tasks=Signal::derive(move ||board.get().later_today) class="later" />
                <BoardColumn title="OVERDUE" tasks=Signal::derive(move ||board.get().overdue) class="overdue" />
                <BoardColumn title="DONE TODAY" tasks=Signal::derive(move ||board.get().done_today) class="done" />
            </section>
            <footer><span><i></i>" Auto-refreshing every 10 seconds"</span><span>"LOCAL · PRIVATE · LIVE"</span></footer>
        </main>
    }
}

#[component]
fn BoardColumn(
    title: &'static str,
    tasks: Signal<Vec<ScoredTask>>,
    class: &'static str,
) -> impl IntoView {
    view! { <section class=format!("board-column {class}")><header><h2>{title}</h2><span>{move ||tasks.get().len()}</span></header><div>
        {move ||tasks.get().into_iter().map(|item|view!{<BoardCard item />}).collect_view()}
    </div></section> }
}

#[component]
fn BoardCard(item: ScoredTask) -> impl IntoView {
    let task = item.context.task;
    view! { <article class="board-card" style=format!("--accent:{}",item.context.organization_color.unwrap_or("#95a095".into()))>
        <div class="board-meta"><Priority value=task.priority /><span>{item.context.organization_name}</span>{task.pinned.then(||view!{<b>"PINNED"</b>})}</div>
        <h3>{task.title}</h3><p>{format!("{} · {}",item.context.project_name,item.context.project_type)}</p>
        <div class="board-bottom">
            {task.time_limit_minutes.map(|m|view!{<span>{format!("LIMIT {m}M")}</span>})}
            {task.due_at.map(|at|view!{<span>{at.format("%-I:%M %p").to_string()}</span>})}
            {task.blocked_reason.map(|reason|view!{<span>{reason}</span>})}
        </div>
    </article> }
}
