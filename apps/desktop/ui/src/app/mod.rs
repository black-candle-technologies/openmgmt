//! Application root: window-mode detection, the persistent app shell
//! (sidebar + top bar + routed content), and the dedicated TV board entry.

pub mod board;
pub mod components;
pub mod dispatch;
pub mod forms;
pub mod pages;
pub mod rank;
pub mod records;
pub mod schedule_modal;
pub mod state;
pub mod tags;
pub mod theme;
pub mod timer;
pub mod updates;
pub mod views;

use chrono::Utc;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use openmgmt_core::Task;
use serde_json::json;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;

use board::BoardView;
use components::Button;
use forms::DrawerHost;
use state::*;

/// Auto-start any scheduled task whose time block is currently active.
///
/// Calls the idempotent `auto_start_due_scheduled_tasks` command: a task is only
/// started once (it then has an active timer and is skipped next tick), so this
/// is safe to run on every refresh. When something is started we surface a small
/// notice and reload so it appears in NOW / In progress immediately. Errors are
/// logged to the console rather than shown, to avoid banner spam on a timer tick.
async fn run_auto_start(state: AppState) {
    match invoke::<Vec<Task>>("auto_start_due_scheduled_tasks", json!({})).await {
        Ok(started) if !started.is_empty() => {
            let message = if started.len() == 1 {
                format!("Auto-started scheduled task: {}", started[0].title)
            } else {
                format!("Auto-started {} scheduled tasks.", started.len())
            };
            state.notice.set(Some(message));
            state.reload().await;
        }
        Ok(_) => {}
        Err(error) => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "[auto-start] auto_start_due_scheduled_tasks failed: {error}"
            )));
        }
    }
}

#[component]
pub fn App() -> impl IntoView {
    theme::initialize();
    let board_only = is_board_window();
    let state = AppState::new();
    let now = RwSignal::new(Utc::now());

    // 1s wall clock, shared by every surface.
    let clock_refresh = Interval::new(1_000, move || now.set(Utc::now()));
    clock_refresh.forget();

    if board_only {
        // Dedicated TV board: load and auto-refresh *only* the board state.
        // It must not depend on the full snapshot (organizations/projects/tasks)
        // so a single failing list query can never blank the board window.
        log_board_diagnostics();
        state.refresh_board();
        let board_refresh = Interval::new(10_000, move || state.refresh_board());
        board_refresh.forget();

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

    updates::Updates::initialize();

    // Main app: load the full snapshot and refresh it every 10s.
    state.refresh();
    let snapshot_refresh = Interval::new(10_000, move || state.refresh());
    snapshot_refresh.forget();

    // Auto-start scheduled tasks whose time block is active. Runs once on launch
    // and then on a steady tick so a task scheduled for "now" starts without the
    // user lifting a finger. Only the main window does this (never the TV board)
    // so a task is never started twice. The command is idempotent.
    spawn_local(async move { run_auto_start(state).await });
    let auto_start_tick = Interval::new(15_000, move || {
        spawn_local(async move { run_auto_start(state).await });
    });
    auto_start_tick.forget();

    let page = RwSignal::new(Page::Dashboard);
    provide_context(page);
    provide_context(GlobalSearch(RwSignal::new(String::new())));
    provide_context(AccountStatus(RwSignal::new(false)));
    view! {
        <div class="app-shell">
            <Sidebar state page />
            <div class="app-main">
                <TopBar state page />
                <main class="app-content">
                    <Feedback state />
                    <updates::UpdateBanner />
                    {move || match page.get() {
                        Page::Dashboard => view! { <pages::Dashboard state page /> }.into_any(),
                        Page::DailyOps => view! { <pages::DailyOpsPage state page now /> }.into_any(),
                        Page::Organizations => view! { <pages::OrganizationsPage state page /> }.into_any(),
                        Page::Projects => view! { <pages::ProjectsPage state page /> }.into_any(),
                        Page::Project(id) => view! { <pages::ProjectDetailPage state page id /> }.into_any(),
                        Page::Tasks => view! { <pages::TasksPage state now /> }.into_any(),
                        Page::Schedule => view! { <pages::SchedulePage state now /> }.into_any(),
                        Page::Board => view! { <pages::BoardPage state now /> }.into_any(),
                        Page::AgentSession(task_id, id) => view! { <dispatch::AgentSession state task_id id /> }.into_any(),
                        Page::Sync => view! { <pages::SyncPage /> }.into_any(),
                        Page::Settings => view! { <pages::SettingsPage state /> }.into_any(),
                    }}
                </main>
            </div>
            <DrawerHost state now />
        </div>
    }
    .into_any()
}

#[component]
fn Sidebar(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let signed_in = expect_context::<AccountStatus>().0;
    Effect::new(move |_| {
        let _ = state.synced_at.get();
        spawn_local(async move {
            signed_in.set(
                invoke::<bool>("dispatch_signed_in", json!({}))
                    .await
                    .unwrap_or(false),
            );
        });
    });
    view! {
        <aside class="sidebar">
            <div class="brand">
                <span class="brand-mark">"OM"</span>
                <div class="brand-text"><strong>"OpenMgmt"</strong><small>"OPERATIONS DESK"</small></div>
            </div>
            <nav class="sidebar-nav">
                <p class="sidebar-label">"WORKSPACE"</p>
                <NavButton label="Dashboard" target=Page::Dashboard page />
                <NavButton label="Daily Operations" target=Page::DailyOps page />
                <NavButton label="Tasks" target=Page::Tasks page />
                <NavButton label="Schedule" target=Page::Schedule page />
                <p class="sidebar-label">"STRUCTURE"</p>
                <NavButton label="Projects" target=Page::Projects page />
                <NavButton label="Organizations" target=Page::Organizations page />
                <p class="sidebar-label">"OPERATIONS"</p>
                <NavButton label="Board" target=Page::Board page />
                <NavButton label="Sync" target=Page::Sync page />
                <NavButton label="Settings" target=Page::Settings page />
            </nav>
            <div class="sidebar-foot">
                <span class="local-dot"><i></i>"Local database"</span>
                <button class="nav-button" on:click=move |_| page.set(Page::Sync)>{move || if signed_in.get() { "Black Candle account →" } else { "Sign in →" }}</button>
            </div>
        </aside>
    }
}

#[component]
fn NavButton(label: &'static str, target: Page, page: RwSignal<Page>) -> impl IntoView {
    let active_target = target.clone();
    let is_active = move || {
        let current = page.get();
        current == active_target
            || matches!(
                (&current, &active_target),
                (Page::Project(_), Page::Projects)
            )
    };
    view! {
        <button
            class="nav-button"
            class:active=is_active
            on:click=move |_| page.set(target.clone())
        >
            {label}
        </button>
    }
}

#[component]
fn TopBar(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let search = expect_context::<GlobalSearch>();
    view! {
        <header class="topbar">
            <div class="topbar-lead">
                <h2 class="topbar-title">{move || page.get().title()}</h2>
                <span class="topbar-status" title=move || state.synced_at.get().map(|at| format!("Last updated {}", at.format("%H:%M:%S"))).unwrap_or_default()>
                    {move || if state.loading.get() {
                        view! { <span class="status-pill status-pill-busy"><span class="spinner"></span>"Refreshing"</span> }.into_any()
                    } else if state.error.get().is_some() {
                        view! { <span class="status-pill status-pill-error">"Error"</span> }.into_any()
                    } else {
                        view! { <span class="status-pill status-pill-ok">{state.synced_at.get().map(|at| format!("Updated {} UTC", at.format("%H:%M"))).unwrap_or_else(|| "Ready".into())}</span> }.into_any()
                    }}
                </span>
            </div>
            <div class="topbar-actions">
                <input class="topbar-search" type="search" aria-label="Search tasks" placeholder="Search tasks…" prop:value=move || search.0.get() on:input=move |e| search.0.set(event_target_value(&e)) on:keydown=move |e| { if e.key() == "Enter" { page.set(Page::Tasks); } } />
                <theme::ThemeToggle />
                <button class="btn btn-ghost" title="Refresh" aria-label="Refresh" on:click=move |_| state.refresh()>"↻"</button>
                <Button variant="ghost" on_click=Callback::new(move |_| {
                    spawn_local(async move {
                        match invoke::<()>("open_tv_board_window", serde_json::json!({})).await {
                            Ok(_) => state.notice.set(Some("TV board opened.".into())),
                            Err(error) => state.fail("Could not open TV board", error),
                        }
                    });
                })><span title="Open TV board" aria-label="Open TV board">"▣"</span></Button>
            </div>
        </header>
    }
}

#[component]
fn Feedback(state: AppState) -> impl IntoView {
    view! {
        {move || state.error.get().map(|message| view! {
            <div class="banner banner-error">
                <span>{message}</span>
                <button class="banner-dismiss" on:click=move |_| state.error.set(None)>"Dismiss"</button>
            </div>
        })}
        {move || state.notice.get().map(|message| view! {
            <div class="banner banner-notice">
                <span>{message}</span>
                <button class="banner-dismiss" on:click=move |_| state.notice.set(None)>"Dismiss"</button>
            </div>
        })}
    }
}
