//! Operations board: a shared seven-column layout used both by the dedicated TV
//! window (`BoardView`) and the in-app Board page (`BoardPanel`).

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{BoardState, ScoredTask};

use super::components::PriorityBadge;
use super::state::AppState;

pub fn board_task_count(board: &BoardState) -> usize {
    board.now.len()
        + board.next_up.len()
        + board.due_soon.len()
        + board.waiting_blocked.len()
        + board.later_today.len()
        + board.overdue.len()
        + board.done_today.len()
}

/// The seven-column grid shared by every board surface.
#[component]
pub fn BoardColumns(board: Signal<BoardState>, now: Signal<DateTime<Utc>>) -> impl IntoView {
    view! {
        <div class="board-columns">
            <BoardColumn title="NOW" accent="now" tasks=Signal::derive(move || board.get().now) now />
            <BoardColumn title="NEXT UP" accent="next" tasks=Signal::derive(move || board.get().next_up) now />
            <BoardColumn title="DUE SOON" accent="due" tasks=Signal::derive(move || board.get().due_soon) now />
            <BoardColumn title="WAITING / BLOCKED" accent="waiting" tasks=Signal::derive(move || board.get().waiting_blocked) now />
            <BoardColumn title="LATER TODAY" accent="later" tasks=Signal::derive(move || board.get().later_today) now />
            <BoardColumn title="OVERDUE" accent="overdue" tasks=Signal::derive(move || board.get().overdue) now />
            <BoardColumn title="DONE TODAY" accent="done" tasks=Signal::derive(move || board.get().done_today) now />
        </div>
    }
}

#[component]
fn BoardColumn(
    title: &'static str,
    accent: &'static str,
    tasks: Signal<Vec<ScoredTask>>,
    now: Signal<DateTime<Utc>>,
) -> impl IntoView {
    view! {
        <section class=format!("board-column board-{accent}")>
            <header class="board-column-head">
                <h2>{title}</h2>
                <span class="board-column-count">{move || tasks.get().len()}</span>
            </header>
            <div class="board-column-body">
                {move || {
                    let items = tasks.get();
                    if items.is_empty() {
                        view! { <p class="board-column-empty">"—"</p> }.into_any()
                    } else {
                        items.into_iter().map(|item| view! { <BoardCard item now /> }).collect_view().into_any()
                    }
                }}
            </div>
        </section>
    }
}

#[component]
fn BoardCard(item: ScoredTask, now: Signal<DateTime<Utc>>) -> impl IntoView {
    let task = item.context.task;
    let accent = item
        .context
        .organization_color
        .clone()
        .unwrap_or_else(|| "#95a095".into());
    let project_line = format!("{} · {}", item.context.organization_name, item.context.project_name);
    let started_at = task.started_at;
    let priority = task.priority;
    let title = task.title.clone();
    let pinned = task.pinned;
    let time_limit = task.time_limit_minutes;
    let due_at = task.due_at;
    let blocked_reason = task.blocked_reason.clone();

    view! {
        <article class="board-card" style=format!("--accent:{accent}")>
            <div class="board-card-top">
                <PriorityBadge value=priority />
                {pinned.then(|| view! { <span class="board-pin">"PINNED"</span> })}
                {started_at.map(|at| view! {
                    <span class="board-timer">{move || {
                        let minutes = (now.get() - at).num_minutes().max(0);
                        format!("{minutes}m")
                    }}</span>
                })}
            </div>
            <h3 class="board-card-title">{title}</h3>
            <p class="board-card-project">{project_line}</p>
            <div class="board-card-foot">
                {due_at.map(|at| view! { <span class="board-chip">{at.format("%-I:%M %p").to_string()}</span> })}
                {time_limit.map(|minutes| view! { <span class="board-chip">{format!("LIMIT {minutes}M")}</span> })}
                {blocked_reason.map(|reason| view! { <span class="board-chip board-chip-warn">{reason}</span> })}
            </div>
        </article>
    }
}

/// Full-window TV board. Dark, high-contrast, auto-refreshing.
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
                // Refresh indicator is absolutely positioned so refreshes never
                // shift layout or blank the columns.
                {move || loading.get().then(|| view! { <span class="tv-updating" title="Updating">"●"</span> })}
            </header>
            {move || error.get().map(|message| view! { <div class="tv-message tv-message-error">{message}</div> })}
            {move || (!loading.get() && error.get().is_none() && board_task_count(&board.get()) == 0).then(|| view! {
                <div class="tv-empty">
                    <h2>"No active board tasks"</h2>
                    <p>"Create an active task or run Seed database in the main window."</p>
                </div>
            })}
            <BoardColumns board now />
            <footer class="tv-foot">
                <span class="tv-live"><i></i>" Auto-refreshing every 10 seconds"</span>
                <button class="btn btn-ghost" on:click=move |_| state.refresh()>"Refresh now"</button>
                <span class="tv-tag">"LOCAL · PRIVATE · LIVE"</span>
            </footer>
        </main>
    }
}
