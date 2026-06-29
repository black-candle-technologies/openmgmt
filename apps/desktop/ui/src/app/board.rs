//! Operations board rendered as an ER / hospital-style tracking board: a dense,
//! high-contrast operations table grouped by urgency (Critical/Overdue → Now →
//! Next Up → Due Soon → Waiting → Later → Done), *not* a Kanban column layout.
//!
//! The same `ErBoard` table powers both the embedded in-app Board page and the
//! dedicated full-window TV board (`BoardView`).

use chrono::{DateTime, Duration, Utc};
use leptos::prelude::*;
use openmgmt_core::{BoardState, RecurrenceRule, ScoredTask, Task, TaskStatus};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::{PriorityBadge, priority_label};
use super::state::{
    AppState, fmt_clock_date, fmt_clock_time, fmt_datetime, fmt_time, fmt_time_range, humanize,
    invoke, recurrence_label,
};
use super::tags::TagChip;

/// A scheduled task "starts soon" when its planned start is within this window.
const STARTS_SOON: Duration = Duration::minutes(30);

/// Local time-range label for a task's scheduled block, when it has one.
fn scheduled_range(task: &Task) -> Option<String> {
    match (
        task.scheduled_start_at,
        task.scheduled_end_at,
        task.scheduled_at,
    ) {
        (Some(start), Some(end), _) => Some(fmt_time_range(start, end)),
        (Some(start), None, _) => Some(fmt_time(start)),
        (None, None, Some(at)) => Some(fmt_time(at)),
        _ => None,
    }
}

/// A recurring task's rule, or `None` for one-off / unset work.
fn recurrence_of(task: &Task) -> Option<RecurrenceRule> {
    task.recurrence_rule
        .filter(|rule| *rule != RecurrenceRule::None)
}

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
    let recurrence = recurrence_of(&task);

    // DUE / WAIT cell: a blocked/waiting reason takes precedence; otherwise the
    // due time (with date for overdue), else a scheduled time, else nothing.
    let is_waiting = matches!(task.status, TaskStatus::Blocked | TaskStatus::Waiting);
    let due_wait = if is_waiting {
        task.blocked_reason
            .clone()
            .unwrap_or_else(|| "Waiting".into())
    } else if let Some(at) = task.due_at {
        if tone == "overdue" {
            fmt_datetime(at)
        } else {
            fmt_time(at)
        }
    } else if let Some(label) = scheduled_range(&task) {
        label
    } else {
        "—".into()
    };
    let due_class = if is_waiting {
        "er-col-due er-due-wait"
    } else {
        "er-col-due"
    };

    view! {
        <div class=format!("er-row er-row-{tone}") style=format!("--org:{org_color}")>
            <span class="er-col-pri"><PriorityBadge value=priority /></span>
            <span class="er-col-task">
                {pinned.then(|| view! { <span class="er-pin" title="Pinned">"★"</span> })}
                <span class="er-task-title" title=title_tooltip>{title}</span>
                {recurrence.map(|rule| view! { <span class="er-recur" title="Repeats">{"↻ "}{recurrence_label(rule)}</span> })}
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

/// DUE / WAIT text for a board card: a blocked/waiting reason wins; otherwise the
/// due time (with date when overdue), else a scheduled time, else a dash.
fn due_wait_text(task: &Task, tone: &str) -> String {
    if matches!(task.status, TaskStatus::Blocked | TaskStatus::Waiting) {
        task.blocked_reason
            .clone()
            .unwrap_or_else(|| "Waiting".into())
    } else if let Some(at) = task.due_at {
        if tone == "overdue" {
            fmt_datetime(at)
        } else {
            fmt_time(at)
        }
    } else if let Some(label) = scheduled_range(task) {
        label
    } else {
        "—".into()
    }
}

/// A board card. `size` is "now" (hero), "next" (secondary), or "lower" (dense
/// row). NOW/NEXT render the full operations card (priority, title, org, project,
/// status, due, active timer, time limit, tags); "lower" renders a compact line.
#[component]
fn BoardCard(
    item: ScoredTask,
    tone: &'static str,
    size: &'static str,
    now: Signal<DateTime<Utc>>,
) -> impl IntoView {
    let context = item.context;
    let task = context.task;
    let priority = task.priority;
    let pinned = task.pinned;
    let title = task.title.clone();
    let title_attr = format!("P{priority} · {} priority", priority_label(priority));
    let status_label = humanize(&task.status.to_string());
    let org = context.organization_name.clone();
    let org_color = context
        .organization_color
        .clone()
        .unwrap_or_else(|| "#7c867c".into());
    let project = context.project_name.clone();
    let tags = task.tags.clone();
    let started_at = task.started_at;
    let limit = task.time_limit_minutes;
    let due = due_wait_text(&task, tone);
    let is_waiting = matches!(task.status, TaskStatus::Blocked | TaskStatus::Waiting);
    let due_class = if is_waiting {
        "bc-due bc-due-wait"
    } else {
        "bc-due"
    };

    // Scheduling indicators: planned time range, repeat badge, and a live
    // "starts soon" / "overdue" cue derived from the card's tone + the clock.
    let sched_range = scheduled_range(&task);
    let recurrence = recurrence_of(&task);
    let sched_start = task.scheduled_start_at.or(task.scheduled_at);
    let starts_soon = move || {
        tone == "later"
            && sched_start.is_some_and(|start| {
                let n = now.get();
                start > n && start - n <= STARTS_SOON
            })
    };
    let overdue_sched = tone == "overdue" && sched_start.is_some();

    let timer = move || {
        started_at.map(|at| {
            view! {
                <span class="bc-timer">
                    {move || format!("{}m", (now.get() - at).num_minutes().max(0))}
                </span>
            }
        })
    };

    if size == "lower" {
        let recurrence_line = recurrence;
        return view! {
            <div class=format!("board-line board-line-{tone}") style=format!("--org:{org_color}")>
                <span class=format!("bc-pri bc-pri-p{priority}") title=title_attr>{format!("P{priority}")}</span>
                <span class="bc-line-title">
                    {pinned.then(|| view! { <span class="bc-pin">"★"</span> })}
                    {title}
                    {recurrence_line.map(|rule| view! { <span class="bc-recur" title="Repeats">{"↻ "}{recurrence_label(rule)}</span> })}
                </span>
                <span class=format!("bc-status bc-status-{tone}")>{status_label}</span>
                <span class=due_class>{due}</span>
                {timer}
            </div>
        }
        .into_any();
    }

    view! {
        <article class=format!("board-card board-card-{size} board-card-{tone}") style=format!("--org:{org_color}")>
            <div class="bc-head">
                <span class=format!("bc-pri bc-pri-p{priority}") title=title_attr>{format!("P{priority}")}</span>
                <span class=format!("bc-status bc-status-{tone}")>{status_label}</span>
                {pinned.then(|| view! { <span class="bc-pin" title="Pinned">"★"</span> })}
            </div>
            <h3 class="bc-title">{title}</h3>
            <div class="bc-meta">
                <span class="bc-org">
                    <span class="bc-org-dot" style=format!("background:{org_color}")></span>
                    {org}
                </span>
                <span class="bc-sep">"·"</span>
                <span class="bc-project">{project}</span>
            </div>
            {(sched_range.is_some() || recurrence.is_some() || overdue_sched).then(|| view! {
                <div class="bc-sched">
                    {sched_range.map(|range| view! { <span class="bc-sched-time" title="Scheduled block">{"◷ "}{range}</span> })}
                    {recurrence.map(|rule| view! { <span class="bc-recur" title="Repeats">{"↻ "}{recurrence_label(rule)}</span> })}
                    {overdue_sched.then(|| view! { <span class="bc-sched-overdue" title="Scheduled block elapsed">"⚠ Overdue"</span> })}
                    {move || starts_soon().then(|| view! { <span class="bc-sched-soon" title="Starts soon">"⏱ Starts soon"</span> })}
                </div>
            })}
            <div class="bc-stats">
                <span class=due_class>{due}</span>
                {timer}
                {limit.map(|minutes| view! { <span class="bc-limit">{format!("limit {minutes}m")}</span> })}
            </div>
            {(!tags.is_empty()).then(|| view! {
                <div class="bc-tags">
                    {tags.into_iter().take(5).map(|tag| view! { <TagChip tag /> }).collect_view()}
                </div>
            })}
        </article>
    }
    .into_any()
}

/// One lower-section box (Overdue, Due Soon, …) with a header and dense rows.
#[component]
fn BoardSection(
    title: &'static str,
    tone: &'static str,
    tasks: Vec<ScoredTask>,
    now: Signal<DateTime<Utc>>,
) -> impl IntoView {
    let count = tasks.len();
    view! {
        <section class=format!("board-box board-box-{tone}")>
            <div class="board-box-head">
                <span class="board-box-title">{title}</span>
                <span class="board-box-count">{count}</span>
            </div>
            <div class="board-box-body">
                {if tasks.is_empty() {
                    view! { <p class="board-pane-clear">"Clear"</p> }.into_any()
                } else {
                    tasks
                        .into_iter()
                        .map(|item| view! { <BoardCard item tone size="lower" now /> })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </section>
    }
}

/// A NOW / NEXT UP hero pane: a big titled box of full board cards.
#[component]
fn BoardPane(
    label: &'static str,
    tone: &'static str,
    size: &'static str,
    pane: &'static str,
    tasks: Vec<ScoredTask>,
    empty: &'static str,
    now: Signal<DateTime<Utc>>,
) -> impl IntoView {
    let count = tasks.len();
    view! {
        <section class=format!("board-pane board-{pane}")>
            <div class=format!("board-pane-head board-pane-head-{tone}")>
                <span class="board-pane-title">{label}</span>
                <span class="board-pane-count">{count}</span>
            </div>
            <div class="board-pane-body">
                {if tasks.is_empty() {
                    view! { <p class="board-pane-clear">{empty}</p> }.into_any()
                } else {
                    tasks
                        .into_iter()
                        .map(|item| view! { <BoardCard item tone size now /> })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </section>
    }
}

/// Full-window TV board: dark, high-contrast, auto-refreshing operations board
/// laid out for distance reading — a dominant NOW pane (top-left) beside NEXT UP
/// (top-right), with the remaining urgency sections boxed below. Type scales with
/// the window via clamp()/vw so it stays readable across a room.
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
                    <div><strong>"OPENMGMT BOARD"</strong><small>"LIVE OPERATIONS"</small></div>
                </div>
                <div class="tv-head-stats">
                    <span class="tv-total">
                        <b>{move || board_active_count(&board.get())}</b>
                        <span>"ACTIVE"</span>
                    </span>
                    <div class="tv-clock">
                        <p>{move || fmt_clock_date(now.get())}</p>
                        <time>{move || fmt_clock_time(now.get())}</time>
                    </div>
                </div>
                <div class="tv-head-actions">
                    {move || loading.get().then(|| view! { <span class="tv-updating" title="Updating">"●"</span> })}
                    <span class="tv-synced">
                        {move || state
                            .synced_at
                            .get()
                            .map(|at| format!("Updated {}", fmt_clock_time(at)))
                            .unwrap_or_else(|| "Updating…".into())}
                    </span>
                    <button class="btn btn-ghost" on:click=move |_| state.refresh_board()>"Refresh"</button>
                    <button class="btn btn-danger-soft" on:click=move |_| {
                        spawn_local(async move {
                            if let Err(error) = invoke::<()>("close_tv_board_window", json!({})).await {
                                state.fail("Could not close board", error);
                            }
                        });
                    }>"Close Board"</button>
                </div>
            </header>

            {move || {
                if let Some(message) = error.get() {
                    return view! {
                        <div class="tv-message tv-message-error">
                            <strong>"Board error: "</strong>{message}
                        </div>
                    }
                    .into_any();
                }
                let board = board.get();
                if !loading.get() && board_task_count(&board) == 0 {
                    return view! {
                        <div class="tv-empty">
                            <h2>"No active board tasks"</h2>
                            <p>"Create your first organization, project, and task to populate the board."</p>
                        </div>
                    }
                    .into_any();
                }
                view! {
                    <div class="tv-board-grid">
                        <BoardPane label="NOW" tone="now" size="now" pane="now"
                            tasks=board.now empty="Nothing active right now." now />
                        <BoardPane label="NEXT UP" tone="next" size="next" pane="next"
                            tasks=board.next_up empty="Nothing queued up next." now />
                        <div class="board-lower">
                            <BoardSection title="OVERDUE" tone="overdue" tasks=board.overdue now />
                            <BoardSection title="DUE SOON" tone="due" tasks=board.due_soon now />
                            <BoardSection title="WAITING / BLOCKED" tone="waiting" tasks=board.waiting_blocked now />
                            <BoardSection title="LATER TODAY" tone="later" tasks=board.later_today now />
                            <BoardSection title="DONE TODAY" tone="done" tasks=board.done_today now />
                        </div>
                    </div>
                }
                .into_any()
            }}

            <footer class="tv-foot">
                <span class="tv-live"><i></i>" Auto-refreshing every 10 seconds"</span>
            </footer>
        </main>
    }
}
