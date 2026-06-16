use leptos::prelude::*;
use openmgmt_core::{ProjectStatus, TaskStatus};

use crate::app::components::*;
use crate::app::records::TaskCard;
use crate::app::state::*;

#[component]
pub fn Dashboard(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let organizations = Signal::derive(move || state.snapshot.get().organizations.len());
    let active_projects = Signal::derive(move || {
        state
            .snapshot
            .get()
            .projects
            .iter()
            .filter(|project| project.status == ProjectStatus::Active)
            .count()
    });
    let open_tasks = Signal::derive(move || {
        state
            .snapshot
            .get()
            .tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Canceled))
            .count()
    });
    let in_progress = Signal::derive(move || state.snapshot.get().board.now.len());
    let overdue = Signal::derive(move || state.snapshot.get().board.overdue.len());
    let due_soon = Signal::derive(move || state.snapshot.get().board.due_soon.len());
    let blocked = Signal::derive(move || state.snapshot.get().board.waiting_blocked.len());

    view! {
        <PageHeader
            eyebrow="COMMAND CENTER"
            title="Operations home"
            description="A live view across every organization, project, and task on this machine."
        >
            <Button variant="ghost" on_click=Callback::new(move |_| state.refresh())>"Refresh"</Button>
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>

        <section class="metric-grid">
            <Metric label="In progress" value=in_progress tone="info" />
            <Metric label="Overdue" value=overdue tone="danger" />
            <Metric label="Due soon" value=due_soon tone="caution" />
            <Metric label="Waiting / blocked" value=blocked tone="warn" />
            <Metric label="Open tasks" value=open_tasks tone="accent" />
            <Metric label="Active projects" value=active_projects tone="neutral" />
            <Metric label="Organizations" value=organizations tone="neutral" />
        </section>

        <Section title="Needs attention now">
            {move || {
                let board = state.snapshot.get().board;
                let tasks = board.now.into_iter()
                    .chain(board.overdue)
                    .chain(board.due_soon)
                    .take(6)
                    .collect::<Vec<_>>();
                if tasks.is_empty() {
                    view! { <EmptyState title="Nothing urgent" hint="No tasks are due, overdue, or in progress right now." /> }.into_any()
                } else {
                    view! {
                        <div class="card-list">
                            {tasks.into_iter().map(|item| {
                                let project = item.context.project_name.clone();
                                view! { <TaskCard state task=item.context.task project_name=project /> }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }
            }}
        </Section>

        <div class="dashboard-split">
            <Section title="Active projects">
                {move || {
                    let projects = state.snapshot.get().projects;
                    let active = projects.into_iter()
                        .filter(|project| project.status == ProjectStatus::Active)
                        .take(6)
                        .collect::<Vec<_>>();
                    if active.is_empty() {
                        view! { <EmptyState title="No active projects" hint="Create a project to start tracking work." /> }.into_any()
                    } else {
                        view! {
                            <ul class="mini-list">
                                {active.into_iter().map(|project| {
                                    let id = project.id.clone();
                                    let kind = project.project_type.to_string();
                                    view! {
                                        <li class="mini-row">
                                            <button class="mini-row-link" on:click=move |_| page.set(Page::Project(id.clone()))>{project.name}</button>
                                            <Badge label=humanize(&kind) tone="neutral" />
                                            <PriorityBadge value=project.priority />
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }
                }}
            </Section>

            <Section title="Waiting / blocked">
                {move || {
                    let board = state.snapshot.get().board;
                    if board.waiting_blocked.is_empty() {
                        view! { <EmptyState title="Nothing blocked" hint="No tasks are waiting or blocked." /> }.into_any()
                    } else {
                        view! {
                            <div class="card-list">
                                {board.waiting_blocked.into_iter().take(5).map(|item| {
                                    let project = item.context.project_name.clone();
                                    view! { <TaskCard state task=item.context.task project_name=project /> }
                                }).collect_view()}
                            </div>
                        }.into_any()
                    }
                }}
            </Section>
        </div>

        <Section title="Quick actions">
            <div class="quick-actions">
                <Button variant="subtle" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateOrganization))>"New organization"</Button>
                <Button variant="subtle" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateProject { organization_id: None }))>"New project"</Button>
                <Button variant="subtle" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
                <Button variant="subtle" on_click=Callback::new(move |_| page.set(Page::Board))>"Open board"</Button>
            </div>
        </Section>
    }
}
