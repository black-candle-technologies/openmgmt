use std::collections::BTreeSet;

use leptos::prelude::*;
use openmgmt_core::ScoredTask;

use crate::app::components::*;
use crate::app::records::TaskCard;
use crate::app::state::*;

#[component]
pub fn TodayPage(state: AppState) -> impl IntoView {
    let board = Signal::derive(move || state.snapshot.get().board);
    let tag_filter = RwSignal::new(String::new());

    // Distinct tags across every board bucket, for the tag filter.
    let all_tags = Signal::derive(move || {
        let board = board.get();
        [
            &board.now,
            &board.overdue,
            &board.due_soon,
            &board.later_today,
            &board.waiting_blocked,
            &board.done_today,
        ]
        .into_iter()
        .flatten()
        .flat_map(|item| item.context.task.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    });

    // Filter a bucket by the active tag (empty = show all).
    let bucket = move |pick: fn(&openmgmt_core::BoardState) -> Vec<ScoredTask>| {
        Signal::derive(move || {
            let tag = tag_filter.get();
            pick(&board.get())
                .into_iter()
                .filter(|item| tag.is_empty() || item.context.task.tags.iter().any(|t| t == &tag))
                .collect::<Vec<_>>()
        })
    };

    view! {
        <PageHeader
            eyebrow="DAILY PLAN"
            title="Today"
            description="Your command center for the day, ordered by urgency and context."
        >
            <Button variant="ghost" on_click=Callback::new(move |_| state.refresh())>"Refresh"</Button>
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>

        <div class="filter-bar">
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

        <div class="today-grid">
            <TodayGroup state title="Now" tone="active" tasks=bucket(|b| b.now.clone()) />
            <TodayGroup state title="Overdue" tone="warn" tasks=bucket(|b| b.overdue.clone()) />
            <TodayGroup state title="Due soon" tone="ready" tasks=bucket(|b| b.due_soon.clone()) />
            <TodayGroup state title="Later today" tone="neutral" tasks=bucket(|b| b.later_today.clone()) />
            <TodayGroup state title="Waiting / blocked" tone="blocked" tasks=bucket(|b| b.waiting_blocked.clone()) />
            <TodayGroup state title="Done today" tone="done" tasks=bucket(|b| b.done_today.clone()) />
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
