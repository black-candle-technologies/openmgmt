use leptos::prelude::*;
use openmgmt_core::ScoredTask;

use crate::app::components::*;
use crate::app::records::TaskCard;
use crate::app::state::*;

#[component]
pub fn TodayPage(state: AppState) -> impl IntoView {
    let board = Signal::derive(move || state.snapshot.get().board);
    view! {
        <PageHeader
            eyebrow="DAILY PLAN"
            title="Today"
            description="Your command center for the day, ordered by urgency and context."
        >
            <Button variant="ghost" on_click=Callback::new(move |_| state.refresh())>"Refresh"</Button>
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>

        <div class="today-grid">
            <TodayGroup state title="Now" tone="active" tasks=Signal::derive(move || board.get().now) />
            <TodayGroup state title="Overdue" tone="warn" tasks=Signal::derive(move || board.get().overdue) />
            <TodayGroup state title="Due soon" tone="ready" tasks=Signal::derive(move || board.get().due_soon) />
            <TodayGroup state title="Later today" tone="neutral" tasks=Signal::derive(move || board.get().later_today) />
            <TodayGroup state title="Waiting / blocked" tone="blocked" tasks=Signal::derive(move || board.get().waiting_blocked) />
            <TodayGroup state title="Done today" tone="done" tasks=Signal::derive(move || board.get().done_today) />
        </div>
    }
}

#[component]
fn TodayGroup(
    state: AppState,
    title: &'static str,
    tone: &'static str,
    tasks: Signal<Vec<ScoredTask>>,
) -> impl IntoView {
    view! {
        <section class=format!("panel today-col today-col-{tone}")>
            <div class="section-head">
                <div class="section-head-title">
                    <h2>{title}</h2>
                    <span class="count-chip">{move || tasks.get().len()}</span>
                </div>
            </div>
            {move || {
                let items = tasks.get();
                if items.is_empty() {
                    view! { <p class="today-clear">"Clear"</p> }.into_any()
                } else {
                    view! {
                        <div class="card-list">
                            {items.into_iter().map(|item| {
                                let project = item.context.project_name.clone();
                                view! { <TaskCard state task=item.context.task project_name=project /> }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }
            }}
        </section>
    }
}
