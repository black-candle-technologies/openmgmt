//! Daily Operations: the live command center for the working day. An active
//! timer hero sits above urgency/intent lenses (Now, Due Soon, Blocked,
//! MVP/Launch, Pinned, Done Today), all backed by `query_tasks`.

use chrono::{DateTime, Duration, Utc};
use leptos::prelude::*;
use openmgmt_core::{RecurrenceRule, TaskStatus, TaskWithContext};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::app::components::*;
use crate::app::state::*;
use crate::app::tags::TagChip;
use crate::app::timer::TimerControls;

fn is_active(status: TaskStatus) -> bool {
    !matches!(status, TaskStatus::Done | TaskStatus::Canceled)
}

#[component]
pub fn DailyOpsPage(
    state: AppState,
    page: RwSignal<Page>,
    now: RwSignal<DateTime<Utc>>,
) -> impl IntoView {
    let now_sig: Signal<DateTime<Utc>> = now.into();
    let tag_filter = RwSignal::new(String::new());

    let results = RwSignal::new(Vec::<TaskWithContext>::new());
    let queried_at = RwSignal::new(Utc::now());
    let querying = RwSignal::new(true);
    let generation = StoredValue::new(0u32);

    // Pull every active task plus those completed today, scored, in one query.
    Effect::new(move |_| {
        let _ = state.synced_at.get();
        let filter = json!({ "include_done": true });
        let sort = json!({ "field": "urgency", "descending": true });
        let token = generation.get_value() + 1;
        generation.set_value(token);
        querying.set(true);
        spawn_local(async move {
            let fetched_at = Utc::now();
            let response = invoke::<Vec<TaskWithContext>>(
                "query_tasks",
                json!({"filter": filter, "sort": sort}),
            )
            .await;
            if generation.get_value() != token {
                return;
            }
            match response {
                Ok(rows) => {
                    results.set(rows);
                    queried_at.set(fetched_at);
                }
                Err(error) => state.fail("Daily operations query failed", error),
            }
            querying.set(false);
        });
    });

    let queried_sig = Signal::derive(move || queried_at.get());

    // Distinct tags for the quick filter.
    let all_tags = Signal::derive(move || {
        let mut tags = results
            .get()
            .iter()
            .flat_map(|row| row.task.tags.iter().cloned())
            .collect::<Vec<_>>();
        tags.sort();
        tags.dedup();
        tags
    });

    // Tag-filtered base set the lenses draw from.
    let filtered = Signal::derive(move || {
        let tag = tag_filter.get();
        results
            .get()
            .into_iter()
            .filter(|row| {
                tag.is_empty()
                    || row
                        .task
                        .tags
                        .iter()
                        .any(|item| item.eq_ignore_ascii_case(&tag))
            })
            .collect::<Vec<_>>()
    });

    // The active timer hero: the running timer if any, else any paused one.
    let active_row = Signal::derive(move || {
        let rows = results.get();
        rows.iter()
            .find(|row| {
                row.active_timer
                    .as_ref()
                    .map(|timer| timer.is_running)
                    .unwrap_or(false)
            })
            .or_else(|| rows.iter().find(|row| row.active_timer.is_some()))
            .cloned()
    });

    let now_bucket = lens(filtered, now_sig, |row, _| {
        row.task.status == TaskStatus::InProgress
    });
    let due_soon_bucket = lens(filtered, now_sig, |row, now| {
        is_active(row.task.status)
            && !matches!(row.task.status, TaskStatus::Blocked | TaskStatus::Waiting)
            && row
                .task
                .due_at
                .is_some_and(|at| at >= now && at <= now + Duration::hours(24))
    });
    let blocked_bucket = lens(filtered, now_sig, |row, _| {
        matches!(row.task.status, TaskStatus::Blocked | TaskStatus::Waiting)
    });
    let tagged_bucket =
        lens(filtered, now_sig, |row, _| {
            is_active(row.task.status)
                && row.task.tags.iter().any(|tag| {
                    tag.eq_ignore_ascii_case("mvp") || tag.eq_ignore_ascii_case("launch")
                })
        });
    let pinned_bucket = lens(filtered, now_sig, |row, _| {
        row.task.pinned && is_active(row.task.status)
    });
    let done_today_bucket = lens(filtered, now_sig, |row, now| {
        row.task.status == TaskStatus::Done
            && row
                .task
                .completed_at
                .is_some_and(|at| at.date_naive() == now.date_naive())
    });

    view! {
        <PageHeader
            eyebrow="COMMAND CENTER"
            title="Daily Operations"
            description="The live desk for today: what's running, what's next, what's stuck, and what's done."
        >
            <Button variant="ghost" on_click=Callback::new(move |_| state.refresh())>"Refresh"</Button>
            <Button variant="subtle" on_click=Callback::new(move |_| page.set(Page::Tasks))>"All tasks"</Button>
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>

        <ActiveTimerHero state row=active_row now=now_sig queried_at=queried_sig />

        <div class="filter-bar daily-quick">
            <button class="view-chip" class:active=move || tag_filter.get().is_empty() on:click=move |_| tag_filter.set(String::new())>"All work"</button>
            <button class="view-chip" class:active=move || tag_filter.get() == "mvp" on:click=move |_| tag_filter.set("mvp".into())>"MVP"</button>
            <button class="view-chip" class:active=move || tag_filter.get() == "launch" on:click=move |_| tag_filter.set("launch".into())>"Launch"</button>
            <button class="view-chip" class:active=move || tag_filter.get() == "bug" on:click=move |_| tag_filter.set("bug".into())>"Bugs"</button>
            <label class="filter-control">
                <span>"Tag"</span>
                <select
                    prop:value=move || tag_filter.get()
                    on:change=move |event| tag_filter.set(event_target_value(&event))
                >
                    <option value="">"All tags"</option>
                    {move || all_tags.get().into_iter().map(|tag| {
                        let value = tag.clone();
                        view! { <option value=value>{tag}</option> }
                    }).collect_view()}
                </select>
            </label>
            {move || querying.get().then(|| view! { <span class="qtask-summary-busy"><span class="spinner"></span>"refreshing"</span> })}
        </div>

        <div class="daily-grid">
            <OpsLens state title="Now" tone="now" rows=now_bucket now=now_sig queried_at=queried_sig empty="Nothing in progress." />
            <OpsLens state title="Due soon" tone="due" rows=due_soon_bucket now=now_sig queried_at=queried_sig empty="Nothing due in the next day." />
            <OpsLens state title="Blocked / waiting" tone="waiting" rows=blocked_bucket now=now_sig queried_at=queried_sig empty="Nothing blocked." />
            <OpsLens state title="MVP / Launch" tone="next" rows=tagged_bucket now=now_sig queried_at=queried_sig empty="No MVP or Launch work tagged." />
            <OpsLens state title="Pinned" tone="pin" rows=pinned_bucket now=now_sig queried_at=queried_sig empty="Nothing pinned." />
            <OpsLens state title="Done today" tone="done" rows=done_today_bucket now=now_sig queried_at=queried_sig empty="No tasks completed yet today." />
        </div>
    }
}

/// Build a reactive bucket from `filtered`, evaluated against the current clock.
fn lens(
    filtered: Signal<Vec<TaskWithContext>>,
    now: Signal<DateTime<Utc>>,
    predicate: fn(&TaskWithContext, DateTime<Utc>) -> bool,
) -> Signal<Vec<TaskWithContext>> {
    Signal::derive(move || {
        let now = now.get();
        filtered
            .get()
            .into_iter()
            .filter(|row| predicate(row, now))
            .collect::<Vec<_>>()
    })
}

#[component]
fn ActiveTimerHero(
    state: AppState,
    row: Signal<Option<TaskWithContext>>,
    now: Signal<DateTime<Utc>>,
    queried_at: Signal<DateTime<Utc>>,
) -> impl IntoView {
    move || {
        match row.get() {
        Some(row) => {
            let task = row.task.clone();
            let title = task.title.clone();
            let project = row.project_name.clone();
            let org = row.organization_name.clone();
            let status = task.status;
            let limit = task.time_limit_minutes;
            let task_id = task.id.clone();
            let active = row.active_timer.clone();
            let running = active.as_ref().map(|t| t.is_running).unwrap_or(false);
            let state_label = if running { "RUNNING" } else { "PAUSED" };
            view! {
                <section class="daily-hero daily-hero-live">
                    <div class="daily-hero-info">
                        <span class=format!("daily-hero-label daily-hero-label-{}", if running { "run" } else { "pause" })>{state_label}" TIMER"</span>
                        <h2 class="daily-hero-title">{title}</h2>
                        <div class="daily-hero-meta">
                            <span>{project}</span>
                            <span class="daily-hero-dot">"·"</span>
                            <span>{org}</span>
                        </div>
                    </div>
                    <TimerControls
                        state now queried_at
                        task_id=task_id
                        status=status
                        time_limit_minutes=limit
                        active=active
                        variant="full"
                    />
                </section>
            }
            .into_any()
        }
        None => view! {
            <section class="daily-hero daily-hero-idle">
                <div>
                    <span class="daily-hero-label">"NO TIMER RUNNING"</span>
                    <p class="daily-hero-hint">"Start a timer on any task below to track focused time."</p>
                </div>
            </section>
        }
        .into_any(),
    }
    }
}

#[component]
fn OpsLens(
    state: AppState,
    #[prop(into)] title: String,
    tone: &'static str,
    rows: Signal<Vec<TaskWithContext>>,
    now: Signal<DateTime<Utc>>,
    queried_at: Signal<DateTime<Utc>>,
    #[prop(into)] empty: String,
) -> impl IntoView {
    view! {
        <section class=format!("panel ops-lens ops-lens-{tone}")>
            <div class="section-head">
                <div class="section-head-title">
                    <h2>{title}</h2>
                    <span class="count-chip">{move || rows.get().len()}</span>
                </div>
            </div>
            {move || {
                let rows = rows.get();
                if rows.is_empty() {
                    view! { <p class="today-clear">{empty.clone()}</p> }.into_any()
                } else {
                    view! {
                        <div class="daily-list">
                            {rows.into_iter().map(|row| view! { <DailyRow state row now queried_at /> }).collect_view()}
                        </div>
                    }.into_any()
                }
            }}
        </section>
    }
}

#[component]
fn DailyRow(
    state: AppState,
    row: TaskWithContext,
    now: Signal<DateTime<Utc>>,
    queried_at: Signal<DateTime<Utc>>,
) -> impl IntoView {
    let task = row.task.clone();
    let edit_task = task.clone();
    let title = task.title.clone();
    let status = task.status;
    let status_str = task.status.to_string();
    let priority = task.priority;
    let pinned = task.pinned;
    let tags = task.tags.clone();
    let task_id = task.id.clone();
    let time_limit = task.time_limit_minutes;
    let active = row.active_timer.clone();
    let project = row.project_name.clone();
    // Scheduling indicators surfaced inline so planned/recurring work reads at a glance.
    let scheduled_label = match (
        task.scheduled_start_at,
        task.scheduled_end_at,
        task.scheduled_at,
    ) {
        (Some(start), Some(end), _) => Some(fmt_time_range(start, end)),
        (Some(start), None, _) => Some(fmt_time(start)),
        (None, None, Some(at)) => Some(fmt_time(at)),
        _ => None,
    };
    let recurrence = task
        .recurrence_rule
        .filter(|rule| *rule != RecurrenceRule::None);
    let org_color = row
        .organization_color
        .clone()
        .unwrap_or_else(|| "#7c867c".into());

    view! {
        <article class="daily-row">
            <span class="daily-row-pri"><PriorityBadge value=priority /></span>
            <div class="daily-row-main">
                <button class="task-row-title" on:click=move |_| state.open_drawer(Drawer::EditTask(edit_task.clone()))>
                    {pinned.then(|| view! { <span class="qtask-pin">"★ "</span> })}
                    {title}
                </button>
                <div class="daily-row-meta">
                    <StatusBadge status=status_str />
                    <span class="er-org-dot" style=format!("background:{org_color}")></span>
                    <span class="daily-row-project">{project}</span>
                    {scheduled_label.map(|label| view! { <span class="task-card-sched">{"◷ "}{label}</span> })}
                    {recurrence.map(|rule| view! { <span class="task-card-recur" title="Repeats">{"↻ "}{recurrence_label(rule)}</span> })}
                    {tags.into_iter().take(3).map(|tag| view! { <TagChip tag /> }).collect_view()}
                </div>
            </div>
            <TimerControls
                state now queried_at
                task_id=task_id
                status=status
                time_limit_minutes=time_limit
                active=active
            />
        </article>
    }
}
