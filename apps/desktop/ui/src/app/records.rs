//! Interactive record components: task rows/cards plus project and organization
//! cards. These bridge presentation (components.rs) and behaviour (state +
//! drawers + task transitions).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{Organization, Project, Task, TaskStatus, TaskWithContext};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::{Badge, EmptyState, LoadingState, PriorityBadge, StatusBadge};
use super::state::*;
use super::tags::TagChip;
use super::timer::TimerControls;

/// Lightweight, table-like list of tasks. `project_label` toggles the project
/// column (hidden on project-detail pages where it is redundant).
#[component]
pub fn TaskTable(
    state: AppState,
    tasks: Signal<Vec<Task>>,
    #[prop(optional)] show_project: bool,
    #[prop(default = "create one above")] empty_hint: &'static str,
) -> impl IntoView {
    view! {
        <div class="task-table">
            <div class="task-row task-row-head">
                <span>"Pri"</span>
                <span>"Task"</span>
                <span>"Status"</span>
                {show_project.then(|| view! { <span>"Project"</span> })}
                <span>"Due"</span>
                <span class="task-row-actions-col">"Actions"</span>
            </div>
            {move || {
                let tasks = tasks.get();
                if tasks.is_empty() {
                    if state.loading.get() {
                        view! { <LoadingState label="Loading tasks…" /> }.into_any()
                    } else {
                        view! { <EmptyState title="No tasks here" hint=format!("Nothing to show — {empty_hint}.") /> }.into_any()
                    }
                } else {
                    let snapshot = state.snapshot.get();
                    tasks.into_iter().map(|task| {
                        let project_name = snapshot.project_name(&task.project_id).unwrap_or_default();
                        view! { <TaskRow state task project_name show_project /> }
                    }).collect_view().into_any()
                }
            }}
        </div>
    }
}

#[component]
fn TaskRow(state: AppState, task: Task, project_name: String, show_project: bool) -> impl IntoView {
    let edit_task = task.clone();
    let title = task.title.clone();
    let status = task.status.to_string();
    let priority = task.priority;
    let pinned = task.pinned;
    let due_label = task
        .due_at
        .map(|at| at.format("%b %-d, %-I:%M %p").to_string())
        .unwrap_or_else(|| "—".into());
    let limit = task.time_limit_minutes;
    let elapsed = task
        .started_at
        .map(|at| (Utc::now() - at).num_minutes().max(0));
    let tags = task.tags.clone();

    view! {
        <div class="task-row">
            <span class="task-row-pri"><PriorityBadge value=priority /></span>
            <div class="task-row-main">
                <button class="task-row-title" on:click=move |_| state.open_drawer(Drawer::EditTask(edit_task.clone()))>
                    {title}
                </button>
                <div class="task-row-tags">
                    {pinned.then(|| view! { <Badge label="pinned" tone="ready" /> })}
                    {elapsed.map(|minutes| view! { <span class="task-row-timer">{format!("{minutes}m active")}</span> })}
                    {limit.map(|minutes| view! { <Badge label=format!("limit {minutes}m") tone="muted" /> })}
                    {tags.into_iter().map(|tag| view! { <TagChip tag /> }).collect_view()}
                </div>
            </div>
            <span class="task-row-status"><StatusBadge status /></span>
            {show_project.then(|| view! { <span class="task-row-project">{project_name.clone()}</span> })}
            <span class="task-row-due">{due_label}</span>
            <span class="task-row-actions"><TaskActions state task /></span>
        </div>
    }
}

/// Rich, query-backed task table used on the Tasks page. Unlike `TaskTable`
/// (which renders plain snapshot tasks), each row here carries organization,
/// urgency score, and live timer state from `query_tasks`.
#[component]
pub fn QueryTaskTable(
    state: AppState,
    rows: Signal<Vec<TaskWithContext>>,
    now: Signal<DateTime<Utc>>,
    queried_at: Signal<DateTime<Utc>>,
    loading: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="qtask-table">
            <div class="qtask-row qtask-row-head">
                <span>"PRI"</span>
                <span>"TASK"</span>
                <span>"STATUS"</span>
                <span>"ORG"</span>
                <span>"PROJECT"</span>
                <span>"DUE"</span>
                <span class="qtask-urg-col">"URG"</span>
                <span class="qtask-timer-col">"TIMER"</span>
            </div>
            {move || {
                let rows = rows.get();
                if rows.is_empty() {
                    if loading.get() {
                        view! { <LoadingState label="Loading tasks…" /> }.into_any()
                    } else {
                        view! { <EmptyState title="No matching tasks" hint="Adjust the filters or choose another saved view." /> }.into_any()
                    }
                } else {
                    rows.into_iter()
                        .map(|row| view! { <QueryTaskRow state row now queried_at /> })
                        .collect_view()
                        .into_any()
                }
            }}
        </div>
    }
}

#[component]
fn QueryTaskRow(
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
    let cancel_id = task.id.clone();
    let cancel_title = task.title.clone();
    let active = row.active_timer.clone();
    let due_label = task
        .due_at
        .map(|at| at.format("%b %-d, %-I:%M %p").to_string())
        .unwrap_or_else(|| "—".into());
    let org_name = row.organization_name.clone();
    let org_color = row
        .organization_color
        .clone()
        .unwrap_or_else(|| "#7c867c".into());
    let project_name = row.project_name.clone();
    let urgency = row.urgency_score;
    let can_cancel = !matches!(status, TaskStatus::Canceled);

    view! {
        <div class="qtask-row">
            <span class="qtask-pri"><PriorityBadge value=priority /></span>
            <div class="qtask-main">
                <button class="task-row-title" on:click=move |_| state.open_drawer(Drawer::EditTask(edit_task.clone()))>
                    {pinned.then(|| view! { <span class="qtask-pin" title="Pinned">"★ "</span> })}
                    {title}
                </button>
                {(!tags.is_empty()).then(|| view! {
                    <div class="qtask-tags">
                        {tags.into_iter().take(5).map(|tag| view! { <TagChip tag /> }).collect_view()}
                    </div>
                })}
            </div>
            <span class="qtask-status"><StatusBadge status=status_str /></span>
            <span class="qtask-org">
                <span class="er-org-dot" style=format!("background:{org_color}")></span>
                <span class="qtask-org-name">{org_name}</span>
            </span>
            <span class="qtask-project">{project_name}</span>
            <span class="qtask-due">{due_label}</span>
            <span class="qtask-urgency qtask-urg-col" title="Urgency score">{urgency}</span>
            <span class="qtask-timer qtask-timer-col">
                <TimerControls
                    state now queried_at
                    task_id=task_id
                    status=status
                    time_limit_minutes=time_limit
                    active=active
                />
                {can_cancel.then(|| view! {
                    <button class="btn-link-danger qtask-cancel" title="Cancel task" on:click=move |_| {
                        if !confirmed(&format!("Cancel task {cancel_title}?")) { return; }
                        let id = cancel_id.clone();
                        spawn_local(async move {
                            finish_action(state, invoke::<Task>("cancel_task", json!({"id":id})).await, "Task canceled.", "Cancel task failed").await;
                        });
                    }>"✕"</button>
                })}
            </span>
        </div>
    }
}

/// Compact task card for dashboard / today lists.
#[component]
pub fn TaskCard(
    state: AppState,
    task: Task,
    #[prop(default = String::new())] project_name: String,
) -> impl IntoView {
    let edit_task = task.clone();
    let title = task.title.clone();
    let status = task.status.to_string();
    let priority = task.priority;
    let elapsed = task
        .started_at
        .map(|at| (Utc::now() - at).num_minutes().max(0));
    let limit = task.time_limit_minutes;
    let due_label = task.due_at.map(|at| at.format("%-I:%M %p").to_string());
    let has_project = !project_name.is_empty();
    let tags = task.tags.clone();

    view! {
        <article class="task-card">
            <span class="task-card-pri"><PriorityBadge value=priority /></span>
            <div class="task-card-body">
                <button class="task-card-title" on:click=move |_| state.open_drawer(Drawer::EditTask(edit_task.clone()))>
                    {title}
                </button>
                <div class="task-card-meta">
                    <StatusBadge status />
                    {has_project.then(|| view! { <span class="task-card-project">{project_name.clone()}</span> })}
                    {due_label.map(|due| view! { <span class="task-card-chip">{due}</span> })}
                    {elapsed.map(|minutes| view! { <span class="task-row-timer">{format!("{minutes}m")}</span> })}
                    {limit.map(|minutes| view! { <span class="task-card-chip">{format!("limit {minutes}m")}</span> })}
                    {tags.into_iter().take(4).map(|tag| view! { <TagChip tag /> }).collect_view()}
                </div>
            </div>
            <TaskActions state task />
        </article>
    }
}

/// Action cluster shared by rows and cards: start / done / block / unblock /
/// cancel. Calls the same Tauri commands as before.
#[component]
fn TaskActions(state: AppState, task: Task) -> impl IntoView {
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
    let can_unblock = matches!(task.status, TaskStatus::Blocked | TaskStatus::Waiting);
    let can_block = !matches!(
        task.status,
        TaskStatus::Blocked | TaskStatus::Waiting | TaskStatus::Done | TaskStatus::Canceled
    );

    view! {
        <div class="task-actions">
            {can_start.then(|| view! {
                <button class="btn btn-subtle" title="Start" on:click=move |_| {
                    let id = start_id.clone();
                    spawn_local(async move {
                        finish_action(state, invoke::<Task>("start_task", json!({"id":id})).await, "Task started.", "Start task failed").await;
                    });
                }>"Start"</button>
            })}
            {can_complete.then(|| view! {
                <button class="btn btn-primary" title="Complete" on:click=move |_| {
                    let id = done_id.clone();
                    spawn_local(async move {
                        finish_action(state, invoke::<Task>("complete_task", json!({"id":id})).await, "Task completed.", "Complete task failed").await;
                    });
                }>"Done"</button>
            })}
            {can_unblock.then(|| view! {
                <button class="btn btn-subtle" title="Unblock" on:click=move |_| {
                    let id = unblock_id.clone();
                    spawn_local(async move {
                        finish_action(state, invoke::<Task>("unblock_task", json!({"id":id})).await, "Task unblocked.", "Unblock task failed").await;
                    });
                }>"Unblock"</button>
            })}
            {can_block.then(|| view! {
                <button class="btn btn-subtle" title="Block" on:click=move |_| {
                    let id = block_id.clone();
                    let reason = block_reason.clone();
                    spawn_local(async move {
                        finish_action(state, invoke::<Task>("block_task", json!({"id":id,"reason":reason})).await, "Task blocked.", "Block task failed").await;
                    });
                }>"Block"</button>
            })}
            <button class="btn-link-danger" title="Cancel task" on:click=move |_| {
                if !confirmed(&format!("Cancel task {cancel_title}?")) { return; }
                let id = cancel_id.clone();
                spawn_local(async move {
                    finish_action(state, invoke::<Task>("cancel_task", json!({"id":id})).await, "Task canceled.", "Cancel task failed").await;
                });
            }>"✕"</button>
        </div>
    }
}

/// Project card used in the projects grid.
#[component]
pub fn ProjectCard(state: AppState, project: Project, page: RwSignal<Page>) -> impl IntoView {
    let open_id = project.id.clone();
    let edit_project = project.clone();
    let name = project.name.clone();
    let description = project
        .description
        .clone()
        .unwrap_or_else(|| "No description yet.".into());
    let project_type = project.project_type.to_string();
    let status = project.status.to_string();
    let priority = project.priority;

    view! {
        <article class="record-card project-card">
            <div class="record-card-top">
                <StatusBadge status />
                <PriorityBadge value=priority />
            </div>
            <h3 class="record-card-title">{name}</h3>
            <p class="record-card-desc">{description}</p>
            <div class="record-card-foot">
                <Badge label=humanize(&project_type) tone="neutral" />
                <div class="record-card-actions">
                    <button class="btn btn-subtle" on:click=move |_| state.open_drawer(Drawer::EditProject(edit_project.clone()))>"Edit"</button>
                    <button class="btn btn-ghost" on:click=move |_| page.set(Page::Project(open_id.clone()))>"Open"</button>
                </div>
            </div>
        </article>
    }
}

/// Organization card used in the organizations grid.
#[component]
pub fn OrganizationCard(
    state: AppState,
    organization: Organization,
    page: RwSignal<Page>,
) -> impl IntoView {
    let edit_org = organization.clone();
    let accent = organization
        .color
        .clone()
        .unwrap_or_else(|| "#778077".into());
    let icon: String = organization
        .icon
        .clone()
        .unwrap_or_else(|| organization.name.chars().take(2).collect());
    let name = organization.name.clone();
    let description = organization
        .description
        .clone()
        .unwrap_or_else(|| "No description yet.".into());
    let slug = organization.slug.clone();

    // Live project/task counts for this organization, derived from the snapshot.
    let org_id = organization.id.clone();
    let counts = Signal::derive(move || {
        let snapshot = state.snapshot.get();
        let project_ids: HashSet<String> = snapshot
            .projects
            .iter()
            .filter(|project| project.organization_id == org_id)
            .map(|project| project.id.clone())
            .collect();
        let task_count = snapshot
            .tasks
            .iter()
            .filter(|task| project_ids.contains(&task.project_id))
            .count();
        (project_ids.len(), task_count)
    });

    view! {
        <article class="record-card org-card" style=format!("--accent:{accent}")>
            <span class="org-accent-bar"></span>
            <div class="org-card-head">
                <span class="org-icon" style=format!("background:{accent}")>{icon}</span>
                <button class="btn btn-subtle" on:click=move |_| state.open_drawer(Drawer::EditOrganization(edit_org.clone()))>"Edit"</button>
            </div>
            <h3 class="record-card-title">{name}</h3>
            <p class="record-card-desc">{description}</p>
            <div class="org-counts">
                <span class="org-count"><b>{move || counts.get().0}</b>" projects"</span>
                <span class="org-count"><b>{move || counts.get().1}</b>" tasks"</span>
            </div>
            <div class="record-card-foot">
                <small class="org-slug">{format!("/{slug}")}</small>
                <button class="btn btn-ghost" on:click=move |_| page.set(Page::Projects)>"View projects"</button>
            </div>
        </article>
    }
}
