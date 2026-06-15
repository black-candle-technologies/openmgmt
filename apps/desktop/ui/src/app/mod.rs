//! Application root: window-mode detection, the persistent app shell
//! (sidebar + top bar + routed content), and the dedicated TV board entry.

pub mod board;
pub mod components;
pub mod forms;
pub mod pages;
pub mod records;
pub mod state;
pub mod tags;

use chrono::Utc;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use board::BoardView;
use components::Button;
use forms::DrawerHost;
use state::*;

#[component]
pub fn App() -> impl IntoView {
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

    // Main app: load the full snapshot and refresh it every 10s.
    state.refresh();
    let snapshot_refresh = Interval::new(10_000, move || state.refresh());
    snapshot_refresh.forget();

    let page = RwSignal::new(Page::Dashboard);
    view! {
        <div class="app-shell">
            <Sidebar state page />
            <div class="app-main">
                <TopBar state page />
                <main class="app-content">
                    <Feedback state />
                    {move || match page.get() {
                        Page::Dashboard => view! { <pages::Dashboard state page /> }.into_any(),
                        Page::Organizations => view! { <pages::OrganizationsPage state page /> }.into_any(),
                        Page::Projects => view! { <pages::ProjectsPage state page /> }.into_any(),
                        Page::Project(id) => view! { <pages::ProjectDetailPage state page id /> }.into_any(),
                        Page::Tasks => view! { <pages::TasksPage state /> }.into_any(),
                        Page::Today => view! { <pages::TodayPage state /> }.into_any(),
                        Page::Board => view! { <pages::BoardPage state now /> }.into_any(),
                    }}
                </main>
            </div>
            <DrawerHost state />
        </div>
    }
    .into_any()
}

#[component]
fn Sidebar(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let _ = state;
    view! {
        <aside class="sidebar">
            <div class="brand">
                <span class="brand-mark">"OM"</span>
                <div class="brand-text"><strong>"OpenMgmt"</strong><small>"OPERATIONS DESK"</small></div>
            </div>
            <nav class="sidebar-nav">
                <p class="sidebar-label">"WORKSPACE"</p>
                <NavButton label="Dashboard" target=Page::Dashboard page />
                <NavButton label="Organizations" target=Page::Organizations page />
                <NavButton label="Projects" target=Page::Projects page />
                <NavButton label="Tasks" target=Page::Tasks page />
                <NavButton label="Today" target=Page::Today page />
                <NavButton label="Board" target=Page::Board page />
            </nav>
            <div class="sidebar-foot">
                <span class="local-dot"><i></i>"Local database"</span>
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
    view! {
        <header class="topbar">
            <div class="topbar-lead">
                <h2 class="topbar-title">{move || page.get().title()}</h2>
                <span class="topbar-status">
                    {move || if state.loading.get() {
                        view! { <span class="status-pill status-pill-busy"><span class="spinner"></span>"Refreshing"</span> }.into_any()
                    } else if state.error.get().is_some() {
                        view! { <span class="status-pill status-pill-error">"Error"</span> }.into_any()
                    } else {
                        view! { <span class="status-pill status-pill-ok">"Up to date"</span> }.into_any()
                    }}
                </span>
            </div>
            <div class="topbar-actions">
                <Button variant="ghost" on_click=Callback::new(move |_| state.refresh())>"Refresh"</Button>
                <Button variant="subtle" on_click=Callback::new(move |_| {
                    spawn_local(async move {
                        finish_action(
                            state,
                            invoke::<()>("seed_database", json!({})).await,
                            "Database seeded.",
                            "Seed failed",
                        ).await;
                    });
                })>"Seed database"</Button>
                <Button variant="primary" on_click=Callback::new(move |_| {
                    spawn_local(async move {
                        match invoke::<()>("open_tv_board_window", json!({})).await {
                            Ok(_) => state.notice.set(Some("TV board opened.".into())),
                            Err(error) => state.fail("Could not open TV board", error),
                        }
                    });
                })>"Open TV Board"</Button>
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
