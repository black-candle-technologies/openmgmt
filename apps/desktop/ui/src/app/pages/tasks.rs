use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{Task, TaskStatus};

use crate::app::components::*;
use crate::app::records::TaskTable;
use crate::app::state::*;

/// Sort orders offered on the Tasks page.
#[derive(Clone, Copy, PartialEq)]
enum TaskSort {
    Urgency,
    Priority,
    Due,
    Status,
    Project,
    Tag,
}

impl TaskSort {
    fn from_key(key: &str) -> Self {
        match key {
            "priority" => Self::Priority,
            "due" => Self::Due,
            "status" => Self::Status,
            "project" => Self::Project,
            "tag" => Self::Tag,
            _ => Self::Urgency,
        }
    }
}

/// Higher = more pressing; used for the urgency and status sorts.
fn status_rank(status: TaskStatus) -> i32 {
    match status {
        TaskStatus::InProgress => 7,
        TaskStatus::Blocked => 6,
        TaskStatus::Waiting => 5,
        TaskStatus::Ready => 4,
        TaskStatus::Scheduled => 3,
        TaskStatus::Inbox => 2,
        TaskStatus::Backlog => 1,
        TaskStatus::Done => 0,
        TaskStatus::Canceled => -1,
    }
}

/// Sortable due key: tasks with a due date sort before those without.
fn due_key(task: &Task) -> i64 {
    task.due_at
        .map(|at| at.timestamp_millis())
        .unwrap_or(i64::MAX)
}

fn is_overdue(task: &Task, now: DateTime<Utc>) -> bool {
    task.due_at.is_some_and(|at| at < now)
        && !matches!(task.status, TaskStatus::Done | TaskStatus::Canceled)
}

fn sort_tasks(tasks: &mut [Task], sort: TaskSort, snapshot: &Snapshot) {
    match sort {
        TaskSort::Urgency => {
            let now = Utc::now();
            tasks.sort_by(|a, b| {
                b.pinned
                    .cmp(&a.pinned)
                    .then_with(|| is_overdue(b, now).cmp(&is_overdue(a, now)))
                    .then_with(|| status_rank(b.status).cmp(&status_rank(a.status)))
                    .then_with(|| due_key(a).cmp(&due_key(b)))
                    .then_with(|| b.priority.cmp(&a.priority))
            });
        }
        TaskSort::Priority => tasks.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| due_key(a).cmp(&due_key(b)))
        }),
        TaskSort::Due => tasks.sort_by(|a, b| due_key(a).cmp(&due_key(b))),
        TaskSort::Status => tasks.sort_by(|a, b| {
            status_rank(b.status)
                .cmp(&status_rank(a.status))
                .then_with(|| b.priority.cmp(&a.priority))
        }),
        TaskSort::Project => tasks.sort_by(|a, b| {
            let pa = snapshot
                .project_name(&a.project_id)
                .unwrap_or_default()
                .to_lowercase();
            let pb = snapshot
                .project_name(&b.project_id)
                .unwrap_or_default()
                .to_lowercase();
            pa.cmp(&pb).then_with(|| b.priority.cmp(&a.priority))
        }),
        TaskSort::Tag => tasks.sort_by(|a, b| {
            let ta = a.tags.first().map(|tag| tag.to_lowercase());
            let tb = b.tags.first().map(|tag| tag.to_lowercase());
            // Tasks with tags sort before those without.
            match (ta, tb) {
                (Some(x), Some(y)) => x.cmp(&y).then_with(|| b.priority.cmp(&a.priority)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => b.priority.cmp(&a.priority),
            }
        }),
    }
}

#[component]
pub fn TasksPage(state: AppState) -> impl IntoView {
    let project_filter = RwSignal::new(String::new());
    let status_filter = RwSignal::new(String::new());
    let tag_filter = RwSignal::new(String::new());
    let sort_by = RwSignal::new(String::from("urgency"));

    // All distinct tags currently in use, for the tag filter dropdown.
    let all_tags = Signal::derive(move || {
        state
            .snapshot
            .get()
            .tasks
            .iter()
            .flat_map(|task| task.tags.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    });

    let filtered = Signal::derive(move || {
        let project = project_filter.get();
        let status = status_filter.get();
        let tag = tag_filter.get();
        let sort = TaskSort::from_key(&sort_by.get());
        let snapshot = state.snapshot.get();
        let mut tasks = snapshot
            .tasks
            .iter()
            .filter(|task| project.is_empty() || task.project_id == project)
            .filter(|task| status.is_empty() || task.status.to_string() == status)
            .filter(|task| tag.is_empty() || task.tags.iter().any(|item| item == &tag))
            .cloned()
            .collect::<Vec<_>>();
        sort_tasks(&mut tasks, sort, &snapshot);
        tasks
    });

    view! {
        <PageHeader
            eyebrow="EXECUTION"
            title="Tasks"
            description="Every task in a scannable table. Filter by project, status, or tag; sort by urgency, priority, due date, status, project, or tag."
        >
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>

        <div class="filter-bar">
            <label class="filter-control">
                <span>"Sort by"</span>
                <select on:change=move |event| sort_by.set(event_target_value(&event))>
                    <option value="urgency">"Urgency"</option>
                    <option value="priority">"Priority"</option>
                    <option value="due">"Due date"</option>
                    <option value="status">"Status"</option>
                    <option value="project">"Project"</option>
                    <option value="tag">"Tag"</option>
                </select>
            </label>
            <label class="filter-control">
                <span>"Project"</span>
                <select on:change=move |event| project_filter.set(event_target_value(&event))>
                    <option value="">"All projects"</option>
                    {move || state.snapshot.get().projects.into_iter().map(|item| view! {
                        <option value=item.id>{item.name}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Status"</span>
                <select on:change=move |event| status_filter.set(event_target_value(&event))>
                    <option value="">"All statuses"</option>
                    {task_status_options().into_iter().map(|(value, label)| view! {
                        <option value=value.to_string()>{label}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Tag"</span>
                <select on:change=move |event| tag_filter.set(event_target_value(&event))>
                    <option value="">"All tags"</option>
                    {move || all_tags.get().into_iter().map(|tag| {
                        let value = tag.clone();
                        view! { <option value=value>{tag}</option> }
                    }).collect_view()}
                </select>
            </label>
        </div>

        <Panel>
            <TaskTable state tasks=filtered show_project=true empty_hint="create one or adjust filters" />
        </Panel>
    }
}
