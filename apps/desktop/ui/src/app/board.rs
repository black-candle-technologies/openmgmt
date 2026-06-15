//! Operations board rendered as an ER / hospital-style tracking board: a dense,
//! high-contrast operations table grouped by urgency (Critical/Overdue → Now →
//! Next Up → Due Soon → Waiting → Later → Done), *not* a Kanban column layout.
//!
//! The same `ErBoard` table powers both the embedded in-app Board page and the
//! dedicated full-window TV board (`BoardView`).

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{BoardState, ScoredTask, TaskStatus};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::PriorityBadge;
use super::state::{AppState, humanize, invoke};
use super::tags::TagChip;

/// Total tasks across every urgency bucket (includes "done today").
pub fn board_task_count(board: &BoardState) -> usize {
    board.now.len()
        + board.next_up.len()
        + board.due_soon.len()
        + board.waiting_blocked.len()
        + board.later_today.len()
        + board.overdue.len()
        + board.done_today.len()
}

/// Tasks still needing attention (everything except "done today").
pub fn board_active_count(board: &BoardState) -> usize {
    board_task_count(board) - board.done_today.len()
}

/// Urgency buckets in scan order, each tagged with a stable CSS key and label.
/// The key drives the colour of the section band and the per-row status cell.
fn board_groups(board: &BoardState) -> [(&'static str, &'static str, Vec<ScoredTask>); 7] {
    [
        ("overdue", "CRITICAL / OVERDUE", board.overdue.clone()),
        ("now", "NOW", board.now.clone()),
        ("next", "NEXT UP", board.next_up.clone()),
        ("due", "DUE SOON", board.due_soon.clone()),
        (
            "waiting",
            "WAITING / BLOCKED",
            board.waiting_blocked.clone(),
        ),
        ("later", "LATER TODAY", board.later_today.clone()),
        ("done", "DONE TODAY", board.done_today.clone()),
    ]
}

/// Short label for the header summary stat chips.
fn group_short(key: &str) -> &'static str {
    match key {
        "overdue" => "OVERDUE",
        "now" => "NOW",
        "next" => "NEXT",
        "due" => "DUE SOON",
        "waiting" => "WAITING",
        "later" => "LATER",
        "done" => "DONE",
        _ => "",
    }
}

/// The shared ER-style operations table. Renders a single sticky column header
/// followed by colour-coded urgency bands, each band followed by its task rows.
#[component]
pub fn ErBoard(board: Signal<BoardState>, now: Signal<DateTime<Utc>>) -> impl IntoView {
    view! {
        <div class="er-board">
            <div class="er-row er-head-row">
                <span class="er-col-pri">"PRI"</span>
                <span class="er-col-task">"TASK"</span>
                <span class="er-col-org">"ORG"</span>
                <span class="er-col-project">"PROJECT"</span>
                <span class="er-col-tags">"TAGS"</span>
                <span class="er-col-status">"STATUS"</span>
                <span class="er-col-due">"DUE / WAIT"</span>
                <span class="er-col-active">"ACTIVE"</span>
                <span class="er-col-limit">"LIMIT"</span>
                <span class="er-col-type">"TYPE"</span>
            </div>
            {move || {
                board_groups(&board.get())
                    .into_iter()
                    .filter(|(_, _, tasks)| !tasks.is_empty())
                    .map(|(key, title, tasks)| {
                        let count = tasks.len();
                        view! {
                            <div class=format!("er-band er-band-{key}")>
                                <span class="er-band-dot"></span>
                                <span class="er-band-title">{title}</span>
                                <span class="er-band-count">{count}</span>
                            </div>
                            {tasks
                                .into_iter()
                                .map(|item| view! { <ErRow item tone=key now /> })
                                .collect_view()}
                        }
                    })
                    .collect_view()
            }}
        </div>
    }
}

#[component]
fn ErRow(item: ScoredTask, tone: &'static str, now: Signal<DateTime<Utc>>) -> impl IntoView {
    let context = item.context;
    let task = context.task;

    let priority = task.priority;
    let pinned = task.pinned;
    let title = task.title.clone();
    let title_tooltip = task.title.clone();
    let status_label = humanize(&task.status.to_string());
    let org_name = context.organization_name.clone();
    let org_color = context
        .organization_color
        .clone()
        .unwrap_or_else(|| "#7c867c".into());
    let project_name = context.project_name.clone();
    let project_type = humanize(&context.project_type.to_string());
    let tags = task.tags.clone();
    let started_at = task.started_at;
    let limit = task.time_limit_minutes;

    // DUE / WAIT cell: a blocked/waiting reason takes precedence; otherwise the
    // due time (with date for overdue), else a scheduled time, else nothing.
    let is_waiting = matches!(task.status, TaskStatus::Blocked | TaskStatus::Waiting);
    let due_wait = if is_waiting {
        task.blocked_reason
            .clone()
            .unwrap_or_else(|| "Waiting".into())
    } else if let Some(at) = task.due_at {
        if tone == "overdue" {
            at.format("%-m/%-d %-I:%M %p").to_string()
        } else {
            at.format("%-I:%M %p").to_string()
        }
    } else if let Some(at) = task.scheduled_at {
        at.format("%-I:%M %p").to_string()
    } else {
        "—".into()
    };
    let due_class = if is_waiting {
        "er-col-due er-due-wait"
    } else {
        "er-col-due"
    };

    view! {
        <div class=format!("er-row er-row-{tone}")>
            <span class="er-col-pri"><PriorityBadge value=priority /></span>
            <span class="er-col-task">
                {pinned.then(|| view! { <span class="er-pin" title="Pinned">"★"</span> })}
                <span class="er-task-title" title=title_tooltip>{title}</span>
            </span>
            <span class="er-col-org">
                <span class="er-org-dot" style=format!("background:{org_color}")></span>
                <span class="er-org-name">{org_name}</span>
            </span>
            <span class="er-col-project">{project_name}</span>
            <span class="er-col-tags">
                {if tags.is_empty() {
                    view! { <span class="er-dash">"—"</span> }.into_any()
                } else {
                    tags.into_iter()
                        .take(4)
                        .map(|tag| view! { <TagChip tag /> })
                        .collect_view()
                        .into_any()
                }}
            </span>
            <span class=format!("er-col-status er-status er-status-{tone}")>{status_label}</span>
            <span class=due_class>{due_wait}</span>
            <span class="er-col-active">
                {match started_at {
                    Some(at) => view! {
                        <span class="er-timer">
                            {move || format!("{}m", (now.get() - at).num_minutes().max(0))}
                        </span>
                    }.into_any(),
                    None => view! { <span class="er-dash">"—"</span> }.into_any(),
                }}
            </span>
            <span class="er-col-limit">
                {match limit {
                    Some(minutes) => view! { <span class="er-limit">{format!("{minutes}m")}</span> }.into_any(),
                    None => view! { <span class="er-dash">"—"</span> }.into_any(),
                }}
            </span>
            <span class="er-col-type"><span class="er-type">{project_type}</span></span>
        </div>
    }
}

/// Full-window TV board: dark, high-contrast, auto-refreshing ER board with a
/// live header, status summary, empty/error states, refresh and close controls.
///
/// TODO(kiosk): a future wall-mounted "kiosk" mode could request a fullscreen,
/// borderless variant of this window. It is intentionally NOT enabled here — the
/// board always opens as a normal, closable, decorated window.
#[component]
pub fn BoardView(
    board: Signal<BoardState>,
    error: RwSignal<Option<String>>,
    loading: RwSignal<bool>,
    now: RwSignal<DateTime<Utc>>,
    state: AppState,
) -> impl IntoView {
    let now: Signal<DateTime<Utc>> = now.into();
    view! {
        <main class="tv-board">
            <header class="tv-head">
                <div class="tv-brand">
                    <span class="brand-mark">"OM"</span>
                    <div><strong>"OPENMGMT"</strong><small>"LIVE OPERATIONS BOARD"</small></div>
                </div>
                <div class="tv-clock">
                    <p>{move || now.get().format("%A, %B %-d").to_string()}</p>
                    <time>{move || now.get().format("%-I:%M:%S %p").to_string()}</time>
                </div>
                // Absolutely positioned so refreshes never shift layout.
                {move || loading.get().then(|| view! { <span class="tv-updating" title="Updating">"●"</span> })}
            </header>

            <div class="tv-stats">
                <span class="tv-stat tv-stat-total">
                    <b>{move || board_active_count(&board.get())}</b>
                    <span>"ACTIVE"</span>
                </span>
                {move || board_groups(&board.get())
                    .into_iter()
                    .map(|(key, _, tasks)| view! {
                        <span class=format!("tv-stat tv-stat-{key}")>
                            <b>{tasks.len()}</b>
                            <span>{group_short(key)}</span>
                        </span>
                    })
                    .collect_view()}
            </div>

            {move || error.get().map(|message| view! {
                <div class="tv-message tv-message-error">
                    <strong>"Board error: "</strong>{message}
                </div>
            })}
            {move || (!loading.get() && error.get().is_none() && board_task_count(&board.get()) == 0).then(|| view! {
                <div class="tv-empty">
                    <h2>"No active board tasks"</h2>
                    <p>"Create an active task or run Seed database in the main window."</p>
                </div>
            })}

            <div class="tv-board-scroll">
                <ErBoard board now />
            </div>

            <footer class="tv-foot">
                <span class="tv-live"><i></i>" Auto-refreshing every 10 seconds"</span>
                <span class="tv-synced">
                    {move || state
                        .synced_at
                        .get()
                        .map(|at| format!("Updated {}", at.format("%-I:%M:%S %p")))
                        .unwrap_or_else(|| "Updating…".into())}
                </span>
                <span class="tv-foot-actions">
                    <button class="btn btn-ghost" on:click=move |_| state.refresh_board()>"Refresh"</button>
                    <button class="btn btn-danger-soft" on:click=move |_| {
                        spawn_local(async move {
                            if let Err(error) = invoke::<()>("close_tv_board_window", json!({})).await {
                                state.fail("Could not close board", error);
                            }
                        });
                    }>"Close Board"</button>
                </span>
            </footer>
        </main>
    }
}
