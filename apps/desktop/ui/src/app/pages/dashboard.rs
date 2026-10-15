use chrono::Utc;
use leptos::prelude::*;
use openmgmt_core::ProjectStatus;

use crate::app::components::*;
use crate::app::records::TaskCard;
use crate::app::state::*;

#[component]
pub fn Dashboard(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let in_progress = Signal::derive(move || state.snapshot.get().board.now.len());
    let overdue = Signal::derive(move || state.snapshot.get().board.overdue.len());
    let due_soon = Signal::derive(move || state.snapshot.get().board.due_soon.len());
    let blocked = Signal::derive(move || state.snapshot.get().board.waiting_blocked.len());
    view! {
        <PageHeader eyebrow="WORKSPACE" title="Dashboard" description="Your work, at a glance.">
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>
        <section class="metric-grid">
            <Metric label="In progress" value=in_progress tone="info" />
            <Metric label="Overdue" value=overdue tone="danger" />
            <Metric label="Due soon" value=due_soon tone="caution" />
            <Metric label="Blocked & waiting" value=blocked tone="warn" />
        </section>
        <div class="dashboard-split">
            <Section title="Needs attention">
                {move || {
                    let board = state.snapshot.get().board;
                    let mut seen = std::collections::HashSet::new();
                    let tasks = board.overdue.into_iter().chain(board.now).chain(board.due_soon)
                        .filter(|item| seen.insert(item.context.task.id.clone())).take(5).collect::<Vec<_>>();
                    if tasks.is_empty() {
                        view! { <EmptyState title="Nothing urgent" hint="You're all caught up." /> }.into_any()
                    } else {
                        view! { <div class="card-list">{tasks.into_iter().map(|item| view! {
                            <TaskCard state task=item.context.task project_name=item.context.project_name />
                        }).collect_view()}</div> }.into_any()
                    }
                }}
            </Section>
            <div>
                <Section title="Today’s schedule">
                    {move || {
                        let mut tasks = state.snapshot.get().tasks.into_iter().filter(|t| t.scheduled_start_at.or(t.scheduled_at).is_some_and(|at| at.date_naive() == Utc::now().date_naive())).collect::<Vec<_>>();
                        tasks.sort_by_key(|t| t.scheduled_start_at.or(t.scheduled_at));
                        tasks.into_iter().take(5).map(|task| {
                            let title = task.title.clone();
                            view! { <button class="mini-row mini-row-link" on:click=move |_| state.open_drawer(Drawer::EditTask(task.clone()))>{title}</button> }
                        }).collect_view()
                    }}
                    <Button variant="ghost" on_click=Callback::new(move |_| page.set(Page::Schedule))>"View schedule →"</Button>
                </Section>
                <Section title="Active projects">
                    <ul class="mini-list">{move || state.snapshot.get().projects.into_iter()
                        .filter(|p| p.status == ProjectStatus::Active).take(6).map(|project| {
                            let id = project.id;
                            view! { <li class="mini-row"><button class="mini-row-link" on:click=move |_| page.set(Page::Project(id.clone()))>{project.name}</button></li> }
                        }).collect_view()}
                    </ul>
                </Section>
            </div>
        </div>
    }
}
